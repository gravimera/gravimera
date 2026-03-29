use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::gen3d::ai::reuse_groups::Gen3dValidatedReuseGroup;
use crate::gen3d::state::Gen3dDraft;
use crate::object::registry::ObjectDef;

use super::agent_regen_budget::ensure_agent_regen_budget_len;
use super::agent_utils::sanitize_prefix;
use super::artifacts::{append_gen3d_jsonl_artifact, write_gen3d_assembly_snapshot};
use super::job::Gen3dPendingPlanAttempt;
use super::reuse_groups;
use super::schema::{
    AiAimJson, AiAnchorJson, AiAnchorRefJson, AiAttackJson, AiColliderJson, AiContactJson,
    AiMobilityJson, AiPlanArticulationNodeJson, AiPlanAttachmentJson, AiPlanComponentJson,
    AiPlanJsonV1, AiReuseGroupJson,
};
use super::{convert, preserve_plan_policy, Gen3dAiJob, Gen3dPlannedComponent};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyPlanOpsArgsJsonV1 {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    dry_run: bool,
    #[serde(default)]
    base_plan: Option<String>,
    #[serde(default)]
    constraints: Option<ApplyPlanOpsConstraintsJsonV1>,
    ops: Vec<PlanOpJsonV1>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyPlanOpsConstraintsJsonV1 {
    #[serde(default)]
    preserve_existing_components: Option<bool>,
    #[serde(default)]
    preserve_edit_policy: Option<String>,
    #[serde(default)]
    rewire_components: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApplyPlanOpsBasePlan {
    Pending,
    Current,
}

fn parse_apply_plan_ops_base_plan(raw: Option<&str>) -> Result<ApplyPlanOpsBasePlan, String> {
    match raw.unwrap_or("pending").trim() {
        "pending" => Ok(ApplyPlanOpsBasePlan::Pending),
        "current" => Ok(ApplyPlanOpsBasePlan::Current),
        other => Err(format!(
            "Invalid base_plan={other:?}. Expected one of: \"pending\", \"current\"."
        )),
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum PlanOpJsonV1 {
    AddComponent {
        name: String,
        size: [f32; 3],
        #[serde(default)]
        purpose: String,
        #[serde(default)]
        modeling_notes: String,
        #[serde(default)]
        anchors: Vec<AiAnchorJson>,
        #[serde(default)]
        contacts: Vec<AiContactJson>,
        #[serde(default)]
        articulation_nodes: Vec<AiPlanArticulationNodeJson>,
        #[serde(default)]
        attach_to: Option<AiPlanAttachmentJson>,
    },
    RemoveComponent {
        name: String,
    },
    SetAttachTo {
        component: String,
        set_attach_to: Option<AiPlanAttachmentJson>,
    },
    SetAnchor {
        component: String,
        anchor: AiAnchorJson,
    },
    SetAimComponents {
        components: Vec<String>,
    },
    SetAttackMuzzle {
        component: String,
        anchor: String,
    },
    SetReuseGroups {
        reuse_groups: Vec<AiReuseGroupJson>,
    },
    SetMobility {
        mobility: AiMobilityJson,
    },
    SetAttack {
        attack: serde_json::Value,
    },
    SetCollider {
        collider: serde_json::Value,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct OpRejectionJsonV1 {
    index: usize,
    kind: String,
    error: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct OpAppliedJsonV1 {
    index: usize,
    kind: String,
    diff: serde_json::Value,
}

#[derive(Clone, Debug, Default)]
struct PlanOpsApplyState {
    components_added: usize,
    components_removed: usize,
    attachments_set: usize,
    anchors_upserted: usize,
    aim_set: bool,
    muzzle_set: bool,
    reuse_groups_set: bool,
    mobility_set: bool,
    attack_set: bool,
    collider_set: bool,
    touched_components: std::collections::BTreeSet<String>,
}

fn trim_nonempty(field: &str, raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("Missing {field}"));
    }
    Ok(trimmed.to_string())
}

fn validate_vec3_finite(field: &str, v: [f32; 3]) -> Result<(), String> {
    let vec = Vec3::new(v[0], v[1], v[2]);
    if !vec.is_finite() {
        return Err(format!("{field} must be finite"));
    }
    Ok(())
}

fn find_component_idx(plan: &AiPlanJsonV1, name: &str) -> Option<usize> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    plan.components.iter().position(|c| c.name.trim() == name)
}

fn attack_kind_label(attack: Option<&AiAttackJson>) -> &'static str {
    match attack {
        None => "omit",
        Some(AiAttackJson::None) => "none",
        Some(AiAttackJson::Melee { .. }) => "melee",
        Some(AiAttackJson::RangedProjectile { .. }) => "ranged_projectile",
    }
}

fn mobility_kind_label(mobility: &AiMobilityJson) -> &'static str {
    match mobility {
        AiMobilityJson::Static => "static",
        AiMobilityJson::Ground { .. } => "ground",
        AiMobilityJson::Air { .. } => "air",
    }
}

fn collider_kind_label(collider: Option<&AiColliderJson>) -> &'static str {
    match collider {
        None => "omit",
        Some(AiColliderJson::None) => "none",
        Some(AiColliderJson::CircleXz { .. }) => "circle_xz",
        Some(AiColliderJson::AabbXz { .. }) => "aabb_xz",
    }
}

fn plan_summary_json(plan: &AiPlanJsonV1) -> serde_json::Value {
    let mut component_names: Vec<String> = plan
        .components
        .iter()
        .map(|c| c.name.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    component_names.sort();
    component_names.dedup();

    let root_from_field = plan
        .root_component
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let root_from_attach: Vec<String> = plan
        .components
        .iter()
        .filter(|c| c.attach_to.is_none())
        .map(|c| c.name.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let root_component = root_from_field
        .or_else(|| (root_from_attach.len() == 1).then(|| root_from_attach[0].clone()));

    serde_json::json!({
        "components_total": plan.components.len(),
        "component_names_total": component_names.len(),
        "component_names_sample": component_names.iter().take(24).cloned().collect::<Vec<_>>(),
        "root_component": root_component,
        "reuse_groups_total": plan.reuse_groups.len(),
        "has_aim": plan.aim.is_some(),
        "mobility_kind": mobility_kind_label(&plan.mobility),
        "attack_kind": attack_kind_label(plan.attack.as_ref()),
        "collider_kind": collider_kind_label(plan.collider.as_ref()),
    })
}

fn attachment_summary_json(att: Option<&AiPlanAttachmentJson>) -> serde_json::Value {
    match att {
        None => serde_json::Value::Null,
        Some(att) => serde_json::json!({
            "parent": att.parent.trim(),
            "parent_anchor": att.parent_anchor.trim(),
            "child_anchor": att.child_anchor.trim(),
            "has_offset": att.offset.is_some(),
            "has_joint": att.joint.is_some(),
        }),
    }
}

fn find_component_references(
    plan: &AiPlanJsonV1,
    name: &str,
) -> std::collections::BTreeSet<String> {
    let name = name.trim();
    let mut refs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    if name.is_empty() {
        return refs;
    }

    if plan
        .root_component
        .as_ref()
        .map(|s| s.trim() == name)
        .unwrap_or(false)
    {
        refs.insert("root_component".into());
    }
    for comp in plan.components.iter() {
        if let Some(att) = comp.attach_to.as_ref() {
            if att.parent.trim() == name {
                refs.insert("attach_to.parent".into());
            }
        }
    }
    if let Some(aim) = plan.aim.as_ref() {
        if aim.components.iter().any(|c| c.trim() == name) {
            refs.insert("aim.components".into());
        }
    }
    if let Some(attack) = plan.attack.as_ref() {
        if let AiAttackJson::RangedProjectile { muzzle, .. } = attack {
            if muzzle.as_ref().is_some_and(|m| m.component.trim() == name) {
                refs.insert("attack.muzzle.component".into());
            }
        }
    }
    for group in plan.reuse_groups.iter() {
        if group.source.trim() == name {
            refs.insert("reuse_groups.source".into());
        }
        if group.targets.iter().any(|t| t.trim() == name) {
            refs.insert("reuse_groups.targets".into());
        }
    }

    refs
}

fn validate_reuse_groups_op(groups: &[AiReuseGroupJson]) -> Result<(), String> {
    const MAX_GROUPS: usize = 32;
    const MAX_TARGETS_PER_GROUP: usize = 64;
    const MAX_TOTAL_TARGETS: usize = 256;

    if groups.len() > MAX_GROUPS {
        return Err(format!(
            "set_reuse_groups: too many groups ({} > max {MAX_GROUPS})",
            groups.len()
        ));
    }
    let mut total_targets: usize = 0;
    for (idx, group) in groups.iter().enumerate() {
        if group.targets.len() > MAX_TARGETS_PER_GROUP {
            return Err(format!(
                "set_reuse_groups: group[{idx}] has too many targets ({} > max {MAX_TARGETS_PER_GROUP})",
                group.targets.len()
            ));
        }
        total_targets = total_targets.saturating_add(group.targets.len());
        if total_targets > MAX_TOTAL_TARGETS {
            return Err(format!(
                "set_reuse_groups: total targets too large (>{MAX_TOTAL_TARGETS})"
            ));
        }
    }
    Ok(())
}

fn apply_one_op(
    idx: usize,
    op: PlanOpJsonV1,
    plan: &mut AiPlanJsonV1,
    state: &mut PlanOpsApplyState,
) -> Result<OpAppliedJsonV1, OpRejectionJsonV1> {
    match op {
        PlanOpJsonV1::AddComponent {
            name,
            size,
            purpose,
            modeling_notes,
            anchors,
            contacts,
            articulation_nodes,
            attach_to,
        } => {
            let name = match trim_nonempty("name", &name) {
                Ok(v) => v,
                Err(err) => {
                    return Err(OpRejectionJsonV1 {
                        index: idx,
                        kind: "add_component".into(),
                        error: err,
                    });
                }
            };
            if find_component_idx(plan, &name).is_some() {
                return Err(OpRejectionJsonV1 {
                    index: idx,
                    kind: "add_component".into(),
                    error: format!("Component `{name}` already exists."),
                });
            }
            if plan.components.len() >= super::super::GEN3D_MAX_COMPONENTS {
                return Err(OpRejectionJsonV1 {
                    index: idx,
                    kind: "add_component".into(),
                    error: format!(
                        "Cannot add component (max components {} reached).",
                        super::super::GEN3D_MAX_COMPONENTS
                    ),
                });
            }
            if let Err(err) = validate_vec3_finite("size", size) {
                return Err(OpRejectionJsonV1 {
                    index: idx,
                    kind: "add_component".into(),
                    error: err,
                });
            }
            if let Some(att) = attach_to.as_ref() {
                if att.parent.trim().is_empty()
                    || att.parent_anchor.trim().is_empty()
                    || att.child_anchor.trim().is_empty()
                {
                    return Err(OpRejectionJsonV1 {
                        index: idx,
                        kind: "add_component".into(),
                        error: "attach_to has empty fields".into(),
                    });
                }
                if att.parent.trim() == name.as_str() {
                    return Err(OpRejectionJsonV1 {
                        index: idx,
                        kind: "add_component".into(),
                        error: "attach_to.parent cannot equal the component name".into(),
                    });
                }
            }

            plan.components.push(AiPlanComponentJson {
                name: name.clone(),
                purpose,
                modeling_notes,
                size,
                anchors,
                contacts,
                articulation_nodes,
                attach_to,
            });
            state.components_added = state.components_added.saturating_add(1);
            state.touched_components.insert(name.clone());
            Ok(OpAppliedJsonV1 {
                index: idx,
                kind: "add_component".into(),
                diff: serde_json::json!({
                    "added_component": name,
                }),
            })
        }
        PlanOpJsonV1::RemoveComponent { name } => {
            let name = match trim_nonempty("name", &name) {
                Ok(v) => v,
                Err(err) => {
                    return Err(OpRejectionJsonV1 {
                        index: idx,
                        kind: "remove_component".into(),
                        error: err,
                    });
                }
            };
            let Some(comp_idx) = find_component_idx(plan, &name) else {
                return Err(OpRejectionJsonV1 {
                    index: idx,
                    kind: "remove_component".into(),
                    error: format!("Unknown component `{name}`."),
                });
            };
            let refs = find_component_references(plan, &name);
            if !refs.is_empty() {
                return Err(OpRejectionJsonV1 {
                    index: idx,
                    kind: "remove_component".into(),
                    error: format!(
                        "Component `{name}` is still referenced by: {}",
                        refs.iter().take(12).cloned().collect::<Vec<_>>().join(", ")
                    ),
                });
            }
            let removed = plan.components.remove(comp_idx);
            state.components_removed = state.components_removed.saturating_add(1);
            state.touched_components.insert(name.clone());
            Ok(OpAppliedJsonV1 {
                index: idx,
                kind: "remove_component".into(),
                diff: serde_json::json!({
                    "removed_component": removed.name.trim(),
                }),
            })
        }
        PlanOpJsonV1::SetAttachTo {
            component,
            set_attach_to,
        } => {
            let component = match trim_nonempty("component", &component) {
                Ok(v) => v,
                Err(err) => {
                    return Err(OpRejectionJsonV1 {
                        index: idx,
                        kind: "set_attach_to".into(),
                        error: err,
                    });
                }
            };
            let Some(comp_idx) = find_component_idx(plan, &component) else {
                return Err(OpRejectionJsonV1 {
                    index: idx,
                    kind: "set_attach_to".into(),
                    error: format!("Unknown component `{component}`."),
                });
            };
            if let Some(att) = set_attach_to.as_ref() {
                if att.parent.trim().is_empty()
                    || att.parent_anchor.trim().is_empty()
                    || att.child_anchor.trim().is_empty()
                {
                    return Err(OpRejectionJsonV1 {
                        index: idx,
                        kind: "set_attach_to".into(),
                        error: "set_attach_to has empty fields".into(),
                    });
                }
                if att.parent.trim() == component.as_str() {
                    return Err(OpRejectionJsonV1 {
                        index: idx,
                        kind: "set_attach_to".into(),
                        error: "set_attach_to.parent cannot equal the component name".into(),
                    });
                }
            }

            let before = plan.components[comp_idx].attach_to.clone();
            plan.components[comp_idx].attach_to = set_attach_to;
            state.attachments_set = state.attachments_set.saturating_add(1);
            state.touched_components.insert(component.clone());
            Ok(OpAppliedJsonV1 {
                index: idx,
                kind: "set_attach_to".into(),
                diff: serde_json::json!({
                    "component": component,
                    "before": attachment_summary_json(before.as_ref()),
                    "after": attachment_summary_json(plan.components[comp_idx].attach_to.as_ref()),
                }),
            })
        }
        PlanOpJsonV1::SetAnchor { component, anchor } => {
            let component = match trim_nonempty("component", &component) {
                Ok(v) => v,
                Err(err) => {
                    return Err(OpRejectionJsonV1 {
                        index: idx,
                        kind: "set_anchor".into(),
                        error: err,
                    });
                }
            };
            let Some(comp_idx) = find_component_idx(plan, &component) else {
                return Err(OpRejectionJsonV1 {
                    index: idx,
                    kind: "set_anchor".into(),
                    error: format!("Unknown component `{component}`."),
                });
            };
            let anchor_name = match trim_nonempty("anchor.name", &anchor.name) {
                Ok(v) => v,
                Err(err) => {
                    return Err(OpRejectionJsonV1 {
                        index: idx,
                        kind: "set_anchor".into(),
                        error: err,
                    });
                }
            };
            if anchor_name == "origin" {
                return Err(OpRejectionJsonV1 {
                    index: idx,
                    kind: "set_anchor".into(),
                    error: "`origin` is implicit; do not set it explicitly.".into(),
                });
            }
            if !Vec3::new(anchor.pos[0], anchor.pos[1], anchor.pos[2]).is_finite()
                || !Vec3::new(anchor.forward[0], anchor.forward[1], anchor.forward[2]).is_finite()
                || !Vec3::new(anchor.up[0], anchor.up[1], anchor.up[2]).is_finite()
            {
                return Err(OpRejectionJsonV1 {
                    index: idx,
                    kind: "set_anchor".into(),
                    error: "anchor fields must be finite".into(),
                });
            }

            let new_anchor = AiAnchorJson {
                name: anchor_name.clone(),
                pos: anchor.pos,
                forward: anchor.forward,
                up: anchor.up,
            };

            let anchors = &mut plan.components[comp_idx].anchors;
            let mut existed = false;
            for existing in anchors.iter_mut() {
                if existing.name.trim() == anchor_name.as_str() {
                    *existing = new_anchor.clone();
                    existed = true;
                    break;
                }
            }
            if !existed {
                anchors.push(new_anchor);
            }
            state.anchors_upserted = state.anchors_upserted.saturating_add(1);
            state.touched_components.insert(component.clone());
            Ok(OpAppliedJsonV1 {
                index: idx,
                kind: "set_anchor".into(),
                diff: serde_json::json!({
                    "component": component,
                    "anchor": anchor_name,
                    "action": if existed { "updated" } else { "added" },
                }),
            })
        }
        PlanOpJsonV1::SetAimComponents { components } => {
            let mut out: Vec<String> = Vec::new();
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            for raw in components.into_iter() {
                let name = raw.trim();
                if name.is_empty() {
                    continue;
                }
                if seen.insert(name.to_string()) {
                    out.push(name.to_string());
                }
            }
            if out.len() > 64 {
                return Err(OpRejectionJsonV1 {
                    index: idx,
                    kind: "set_aim_components".into(),
                    error: "aim.components is too large (max 64)".into(),
                });
            }

            let before = plan
                .aim
                .as_ref()
                .map(|a| a.components.iter().cloned().take(32).collect::<Vec<_>>())
                .unwrap_or_default();
            match plan.aim.as_mut() {
                Some(aim) => {
                    aim.components = out.clone();
                }
                None => {
                    plan.aim = Some(AiAimJson {
                        max_yaw_delta_degrees: None,
                        components: out.clone(),
                    });
                }
            }
            state.aim_set = true;
            Ok(OpAppliedJsonV1 {
                index: idx,
                kind: "set_aim_components".into(),
                diff: serde_json::json!({
                    "before_components_sample": before,
                    "after_components_sample": out.iter().cloned().take(32).collect::<Vec<_>>(),
                }),
            })
        }
        PlanOpJsonV1::SetAttackMuzzle { component, anchor } => {
            let component = match trim_nonempty("component", &component) {
                Ok(v) => v,
                Err(err) => {
                    return Err(OpRejectionJsonV1 {
                        index: idx,
                        kind: "set_attack_muzzle".into(),
                        error: err,
                    });
                }
            };
            let anchor = match trim_nonempty("anchor", &anchor) {
                Ok(v) => v,
                Err(err) => {
                    return Err(OpRejectionJsonV1 {
                        index: idx,
                        kind: "set_attack_muzzle".into(),
                        error: err,
                    });
                }
            };

            let Some(attack) = plan.attack.as_mut() else {
                return Err(OpRejectionJsonV1 {
                    index: idx,
                    kind: "set_attack_muzzle".into(),
                    error: "Plan has no attack object; cannot set muzzle.".into(),
                });
            };
            let before = match attack {
                AiAttackJson::RangedProjectile { muzzle, .. } => muzzle.clone(),
                _ => None,
            };
            match attack {
                AiAttackJson::RangedProjectile { muzzle, .. } => {
                    *muzzle = Some(AiAnchorRefJson { component, anchor });
                }
                other => {
                    return Err(OpRejectionJsonV1 {
                        index: idx,
                        kind: "set_attack_muzzle".into(),
                        error: format!(
                            "attack.kind is `{}`; expected `ranged_projectile`.",
                            attack_kind_label(Some(other))
                        ),
                    });
                }
            }
            state.muzzle_set = true;
            Ok(OpAppliedJsonV1 {
                index: idx,
                kind: "set_attack_muzzle".into(),
                diff: serde_json::json!({
                    "before": match before.as_ref() {
                        None => serde_json::Value::Null,
                        Some(m) => serde_json::json!({ "component": m.component.trim(), "anchor": m.anchor.trim() }),
                    },
                    "after": match plan.attack.as_ref() {
                        Some(AiAttackJson::RangedProjectile { muzzle, .. }) => match muzzle.as_ref() {
                            None => serde_json::Value::Null,
                            Some(m) => serde_json::json!({ "component": m.component.trim(), "anchor": m.anchor.trim() }),
                        },
                        _ => serde_json::Value::Null,
                    },
                }),
            })
        }
        PlanOpJsonV1::SetReuseGroups { reuse_groups } => {
            if let Err(err) = validate_reuse_groups_op(&reuse_groups) {
                return Err(OpRejectionJsonV1 {
                    index: idx,
                    kind: "set_reuse_groups".into(),
                    error: err,
                });
            }
            let before_groups = plan.reuse_groups.len();
            let after_groups = reuse_groups.len();
            plan.reuse_groups = reuse_groups;
            state.reuse_groups_set = true;
            Ok(OpAppliedJsonV1 {
                index: idx,
                kind: "set_reuse_groups".into(),
                diff: serde_json::json!({
                    "before_groups": before_groups,
                    "after_groups": after_groups,
                }),
            })
        }
        PlanOpJsonV1::SetMobility { mobility } => {
            let before = mobility_kind_label(&plan.mobility);
            plan.mobility = mobility;
            state.mobility_set = true;
            Ok(OpAppliedJsonV1 {
                index: idx,
                kind: "set_mobility".into(),
                diff: serde_json::json!({
                    "before_kind": before,
                    "after_kind": mobility_kind_label(&plan.mobility),
                }),
            })
        }
        PlanOpJsonV1::SetAttack { attack } => {
            let before = attack_kind_label(plan.attack.as_ref());
            let next: Option<AiAttackJson> = if attack.is_null() {
                None
            } else {
                match serde_json::from_value(attack) {
                    Ok(v) => Some(v),
                    Err(err) => {
                        return Err(OpRejectionJsonV1 {
                            index: idx,
                            kind: "set_attack".into(),
                            error: format!("Invalid attack: {err}"),
                        });
                    }
                }
            };
            plan.attack = next;
            state.attack_set = true;
            Ok(OpAppliedJsonV1 {
                index: idx,
                kind: "set_attack".into(),
                diff: serde_json::json!({
                    "before_kind": before,
                    "after_kind": attack_kind_label(plan.attack.as_ref()),
                }),
            })
        }
        PlanOpJsonV1::SetCollider { collider } => {
            let before = collider_kind_label(plan.collider.as_ref());
            let next: Option<AiColliderJson> = if collider.is_null() {
                None
            } else {
                match serde_json::from_value(collider) {
                    Ok(v) => Some(v),
                    Err(err) => {
                        return Err(OpRejectionJsonV1 {
                            index: idx,
                            kind: "set_collider".into(),
                            error: format!("Invalid collider: {err}"),
                        });
                    }
                }
            };
            plan.collider = next;
            state.collider_set = true;
            Ok(OpAppliedJsonV1 {
                index: idx,
                kind: "set_collider".into(),
                diff: serde_json::json!({
                    "before_kind": before,
                    "after_kind": collider_kind_label(plan.collider.as_ref()),
                }),
            })
        }
    }
}

fn apply_ops_inner(
    ops: Vec<PlanOpJsonV1>,
    plan: &mut AiPlanJsonV1,
) -> (
    Vec<OpAppliedJsonV1>,
    Vec<OpRejectionJsonV1>,
    PlanOpsApplyState,
) {
    let mut applied: Vec<OpAppliedJsonV1> = Vec::new();
    let mut rejected: Vec<OpRejectionJsonV1> = Vec::new();
    let mut state = PlanOpsApplyState::default();

    for (idx, op) in ops.into_iter().enumerate() {
        match apply_one_op(idx, op, plan, &mut state) {
            Ok(applied_op) => applied.push(applied_op),
            Err(rejected_op) => rejected.push(rejected_op),
        }
    }

    (applied, rejected, state)
}

fn preserve_error_for_plan_apply(
    tool_id: &str,
    can_preserve_geometry: bool,
    preserve_edit_policy_raw: Option<&str>,
    rewire_components: &[String],
    old_components: &[Gen3dPlannedComponent],
    new_components: &[Gen3dPlannedComponent],
) -> Option<String> {
    if !can_preserve_geometry {
        return None;
    }

    let preserve_edit_policy =
        preserve_plan_policy::parse_preserve_edit_policy(preserve_edit_policy_raw);
    if preserve_edit_policy.is_none() {
        let raw = preserve_edit_policy_raw.unwrap_or("<none>").trim();
        return Some(format!(
            "Invalid constraints.preserve_edit_policy={raw:?}. Expected one of: \"additive\", \"allow_offsets\", \"allow_rewire\"."
        ));
    }
    let preserve_edit_policy =
        preserve_edit_policy.unwrap_or(preserve_plan_policy::PreserveEditPolicy::Additive);

    let old_names: std::collections::HashSet<String> =
        old_components.iter().map(|c| c.name.clone()).collect();
    let new_names: std::collections::HashSet<String> =
        new_components.iter().map(|c| c.name.clone()).collect();
    let mut missing: Vec<String> = old_names
        .difference(&new_names)
        .cloned()
        .collect::<Vec<_>>();
    missing.sort();
    if !missing.is_empty() {
        return Some(format!(
            "{tool_id} preserve_existing_components=true requires the plan to include ALL existing component names. Missing: {missing:?}"
        ));
    }

    if let Some(old_root_name) = old_components
        .iter()
        .find(|c| c.attach_to.is_none())
        .map(|c| c.name.clone())
    {
        let new_root_name = new_components
            .iter()
            .find(|c| c.attach_to.is_none())
            .map(|c| c.name.as_str())
            .unwrap_or("");
        if new_root_name != old_root_name.as_str() {
            return Some(format!(
                "{tool_id} preserve_existing_components=true must keep the same root component. Old root=`{}`, new root=`{}`",
                old_root_name, new_root_name
            ));
        }
    }

    let violations = preserve_plan_policy::validate_preserve_mode_plan_diff(
        old_components,
        new_components,
        preserve_edit_policy,
        rewire_components,
    );
    if violations.is_empty() {
        return None;
    }

    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "{tool_id} preserve_existing_components=true edit_policy={} rejected plan diff:",
        preserve_edit_policy.as_str()
    ));
    for v in violations.iter().take(24) {
        lines.push(format!(
            "- component={} kind={:?} field={} old={} new={}",
            v.component, v.kind, v.field, v.old, v.new
        ));
    }
    if violations.len() > 24 {
        lines.push(format!(
            "- … ({} more)",
            violations.len().saturating_sub(24)
        ));
    }
    lines.push(
        "Hint: Use `apply_draft_ops_v1` to adjust offsets/parts, or re-run `llm_generate_plan_v1` with a broader preserve_edit_policy (and explicit rewire_components for allow_rewire), or disable preserve mode for a full rebuild."
            .into(),
    );
    Some(lines.join("\n"))
}

fn apply_plan_acceptance(
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    preserve_existing_components: bool,
    planned_components: Vec<Gen3dPlannedComponent>,
    notes: String,
    defs: Vec<ObjectDef>,
    rig_move_cycle_m: Option<f32>,
    plan_collider: Option<AiColliderJson>,
    validated_reuse_groups: Vec<Gen3dValidatedReuseGroup>,
    reuse_warnings: Vec<String>,
) -> Result<(), String> {
    let old_components = job.planned_components.clone();
    let can_preserve_geometry = preserve_existing_components
        && !old_components.is_empty()
        && draft.total_non_projectile_primitive_parts() > 0;

    let mut draft_next = draft.clone();
    let mut planned_components = planned_components;

    if can_preserve_geometry {
        // Preserve existing component generation status and motion metadata.
        let mut old_by_name: std::collections::HashMap<&str, &Gen3dPlannedComponent> =
            std::collections::HashMap::new();
        let mut old_component_ids: std::collections::HashSet<u128> =
            std::collections::HashSet::new();
        for comp in old_components.iter() {
            old_by_name.insert(comp.name.as_str(), comp);
            old_component_ids.insert(crate::object::registry::builtin_object_id(&format!(
                "gravimera/gen3d/component/{}",
                comp.name
            )));
        }
        for comp in planned_components.iter_mut() {
            let Some(old) = old_by_name.get(comp.name.as_str()) else {
                continue;
            };
            comp.actual_size = old.actual_size;
            comp.contacts = old.contacts.clone();

            // Preserve anchor frames for existing anchors; allow the plan to add new anchors
            // without shifting existing attachments.
            let mut merged_anchors = old.anchors.clone();
            let mut seen_anchor_names: std::collections::HashSet<String> = merged_anchors
                .iter()
                .map(|a| a.name.as_ref().to_string())
                .collect();
            for a in comp.anchors.iter() {
                if seen_anchor_names.insert(a.name.as_ref().to_string()) {
                    merged_anchors.push(a.clone());
                }
            }
            comp.anchors = merged_anchors;

            if let (Some(new_att), Some(old_att)) =
                (comp.attach_to.as_mut(), old.attach_to.as_ref())
            {
                let same_interface = new_att.parent.trim() == old_att.parent.trim()
                    && new_att.parent_anchor.trim() == old_att.parent_anchor.trim()
                    && new_att.child_anchor.trim() == old_att.child_anchor.trim();
                if same_interface {
                    new_att.animations = old_att.animations.clone();
                    new_att.joint = old_att.joint.clone();
                    new_att.fallback_basis = old_att.fallback_basis;
                    super::attachment_motion_basis::normalize_attachment_motion(
                        &mut new_att.fallback_basis,
                        &mut new_att.animations,
                    );
                    super::attachment_motion_basis::rebase_bases_for_offset_change(
                        old_att.offset,
                        new_att.offset,
                        &mut new_att.fallback_basis,
                        &mut new_att.animations,
                    );
                }
            }
        }

        for comp in planned_components.iter_mut() {
            if let Some(att) = comp.attach_to.as_mut() {
                super::attachment_motion_basis::normalize_attachment_motion(
                    &mut att.fallback_basis,
                    &mut att.animations,
                );
            }
        }

        // Preserve existing geometry: merge plan defs into the draft without overwriting
        // primitive/model parts.
        let mut idx_by_id: std::collections::HashMap<u128, usize> =
            std::collections::HashMap::new();
        for (idx, def) in draft_next.defs.iter().enumerate() {
            idx_by_id.insert(def.object_id, idx);
        }
        for next in defs {
            if let Some(idx) = idx_by_id.get(&next.object_id).copied() {
                let def = &mut draft_next.defs[idx];
                let preserve_size_and_anchors = old_component_ids.contains(&def.object_id)
                    && def.parts.iter().any(|p| {
                        matches!(
                            p.kind,
                            crate::object::registry::ObjectPartKind::Primitive { .. }
                                | crate::object::registry::ObjectPartKind::Model { .. }
                        )
                    });

                let parts = std::mem::take(&mut def.parts);
                let old_size = def.size;
                let old_anchors = std::mem::take(&mut def.anchors);
                let next_size = next.size;
                let next_anchors = next.anchors;

                def.label = next.label;
                def.ground_origin_y = next.ground_origin_y;
                def.collider = next.collider;
                def.interaction = next.interaction;
                def.aim = next.aim;
                def.mobility = next.mobility;
                def.minimap_color = next.minimap_color;
                def.health_bar_offset_y = next.health_bar_offset_y;
                def.enemy = next.enemy;
                def.muzzle = next.muzzle;
                def.projectile = next.projectile;
                def.attack = next.attack;

                if preserve_size_and_anchors {
                    def.size = old_size;
                    let mut merged_anchors = old_anchors;
                    let mut seen_anchor_names: std::collections::HashSet<String> = merged_anchors
                        .iter()
                        .map(|a| a.name.as_ref().to_string())
                        .collect();
                    for a in next_anchors.iter() {
                        if seen_anchor_names.insert(a.name.as_ref().to_string()) {
                            merged_anchors.push(a.clone());
                        }
                    }
                    def.anchors = merged_anchors;
                } else {
                    def.size = next_size;
                    def.anchors = next_anchors;
                }

                def.parts = parts;
            } else {
                draft_next.defs.push(next);
            }
        }

        convert::sync_attachment_tree_to_defs(&planned_components, &mut draft_next)
            .map_err(|err| format!("Failed to sync attachments after plan merge: {err}"))?;
        convert::update_root_def_from_planned_components(
            &planned_components,
            &plan_collider,
            &mut draft_next,
        );
    } else {
        draft_next.defs = defs;
    }

    if !can_preserve_geometry {
        for comp in planned_components.iter_mut() {
            if let Some(att) = comp.attach_to.as_mut() {
                super::attachment_motion_basis::normalize_attachment_motion(
                    &mut att.fallback_basis,
                    &mut att.animations,
                );
            }
        }
        convert::sync_attachment_tree_to_defs(&planned_components, &mut draft_next).map_err(
            |err| format!("Failed to sync attachments after base-slot normalization: {err}"),
        )?;
        convert::update_root_def_from_planned_components(
            &planned_components,
            &plan_collider,
            &mut draft_next,
        );
    }

    // Commit job + draft.
    job.planned_components = planned_components;
    job.assembly_notes = notes;
    job.rig_move_cycle_m = rig_move_cycle_m;
    job.plan_collider = plan_collider;
    job.reuse_groups = validated_reuse_groups;
    job.reuse_group_warnings = reuse_warnings;
    job.plan_hash = super::compute_gen3d_plan_hash(
        &job.assembly_notes,
        job.rig_move_cycle_m,
        &job.planned_components,
    );

    job.component_queue.clear();
    job.component_queue_pos = 0;
    job.component_attempts
        .resize(job.planned_components.len(), 0);
    job.component_in_flight.clear();
    ensure_agent_regen_budget_len(job);

    job.preserve_existing_components_mode = preserve_existing_components;

    job.agent.workspaces.clear();
    job.agent.active_workspace_id = "main".to_string();
    job.agent.next_workspace_seq = 1;
    job.agent.rendered_since_last_review = false;
    job.agent.last_render_blob_ids.clear();
    job.agent.last_render_assembly_rev = None;
    job.agent.pending_regen_component_indices.clear();
    job.agent
        .pending_regen_component_indices_skipped_due_to_budget
        .clear();
    job.agent
        .pending_regen_component_indices_blocked_due_to_qa_gate
        .clear();
    job.agent.pending_llm_repair_attempt = 0;

    if can_preserve_geometry {
        job.assembly_rev = job.assembly_rev.saturating_add(1);
    } else {
        job.assembly_rev = 0;
    }

    *draft = draft_next;

    if let Some(dir) = job.step_dir_path() {
        write_gen3d_assembly_snapshot(Some(dir), &job.planned_components);
    }

    job.pending_plan_attempt = None;
    Ok(())
}

pub(super) fn apply_plan_ops_v1(
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    call_id: Option<&str>,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let mut args: ApplyPlanOpsArgsJsonV1 = serde_json::from_value(args_json)
        .map_err(|err| format!("Invalid apply_plan_ops_v1 args JSON: {err}"))?;
    if args.version == 0 {
        args.version = 1;
    }
    if args.version != 1 {
        return Err(format!(
            "Unsupported apply_plan_ops_v1 version {} (expected 1)",
            args.version
        ));
    }
    if args.ops.len() > 64 {
        return Err("apply_plan_ops_v1: too many ops (max 64)".into());
    }

    let base_plan = parse_apply_plan_ops_base_plan(args.base_plan.as_deref())
        .map_err(|err| format!("apply_plan_ops_v1: {err}"))?;

    let mut pending_before = match base_plan {
        ApplyPlanOpsBasePlan::Pending => job.pending_plan_attempt.clone().ok_or_else(|| {
            "apply_plan_ops_v1: no pending rejected plan attempt. Set base_plan=\"current\" to patch the current accepted plan."
                .to_string()
        })?,
        ApplyPlanOpsBasePlan::Current => {
            if job.planned_components.is_empty() {
                return Err(
                    "apply_plan_ops_v1: no accepted plan to patch. Run llm_generate_plan_v1 first (or patch a pending rejected plan attempt with base_plan=\"pending\")."
                        .into(),
                );
            }

            let plan_json = super::plan_tools::build_preserve_mode_plan_template_json_v8(
                draft,
                &job.planned_components,
                &job.assembly_notes,
                job.plan_collider.as_ref(),
                job.rig_move_cycle_m,
                &job.reuse_groups,
            )?;
            let plan: AiPlanJsonV1 = serde_json::from_value(plan_json)
                .map_err(|err| format!("apply_plan_ops_v1: failed to parse current plan snapshot: {err}"))?;

            let mut existing_component_names: Vec<String> =
                job.planned_components.iter().map(|c| c.name.clone()).collect();
            existing_component_names.sort();
            existing_component_names.dedup();
            let existing_root_component = job
                .planned_components
                .iter()
                .find(|c| c.attach_to.is_none())
                .map(|c| c.name.clone());

            Gen3dPendingPlanAttempt {
                call_id: call_id.unwrap_or("apply_plan_ops_v1").to_string(),
                error: "(patch current accepted plan)".into(),
                preserve_existing_components: job.preserve_existing_components_mode,
                preserve_edit_policy: None,
                rewire_components: Vec::new(),
                existing_component_names,
                existing_root_component,
                plan,
            }
        }
    };

    if let Some(constraints) = args.constraints.as_ref() {
        if let Some(preserve_existing_components) = constraints.preserve_existing_components {
            pending_before.preserve_existing_components = preserve_existing_components;
        }
        if let Some(raw) = constraints.preserve_edit_policy.as_deref() {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                pending_before.preserve_edit_policy = Some(trimmed.to_string());
            }
        }
        if let Some(rewire_components) = constraints.rewire_components.as_ref() {
            pending_before.rewire_components = rewire_components
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }

    let assembly_rev_before = job.assembly_rev();
    let plan_before = pending_before.plan.clone();
    let summary_before = plan_summary_json(&plan_before);

    let mut plan_after = plan_before.clone();
    let (applied_ops, rejected_ops, state) = apply_ops_inner(args.ops, &mut plan_after);

    let summary_after = plan_summary_json(&plan_after);
    let diff_summary = serde_json::json!({
        "components": {
            "added": state.components_added,
            "removed": state.components_removed,
        },
        "attachments_set": state.attachments_set,
        "anchors_upserted": state.anchors_upserted,
        "aim_set": state.aim_set,
        "mobility_set": state.mobility_set,
        "attack_set": state.attack_set,
        "attack_muzzle_set": state.muzzle_set,
        "collider_set": state.collider_set,
        "reuse_groups_set": state.reuse_groups_set,
        "touched_components": state.touched_components.iter().cloned().take(32).collect::<Vec<_>>(),
    });

    // Re-run semantic validation after ops (and optionally accept the plan).
    let preserve_existing_components = pending_before.preserve_existing_components;
    let preserve_edit_policy_raw = pending_before.preserve_edit_policy.as_deref();
    let rewire_components = pending_before.rewire_components.clone();

    let mut accepted = false;
    let mut still_pending = true;
    let mut new_errors: Vec<serde_json::Value> = Vec::new();
    let mut committed = false;

    match convert::ai_plan_to_initial_draft_defs(plan_after.clone()) {
        Ok((planned_components, notes, defs)) => {
            let rig_move_cycle_m = plan_after
                .rig
                .as_ref()
                .and_then(|r| r.move_cycle_m)
                .filter(|v| v.is_finite())
                .map(f32::abs)
                .filter(|v| *v > 1e-3);
            let plan_collider = plan_after.collider.clone();
            let (validated_reuse_groups, reuse_warnings) =
                reuse_groups::validate_reuse_groups(&plan_after.reuse_groups, &planned_components);

            let can_preserve_geometry = preserve_existing_components
                && !job.planned_components.is_empty()
                && draft.total_non_projectile_primitive_parts() > 0;
            if let Some(err) = preserve_error_for_plan_apply(
                "apply_plan_ops_v1",
                can_preserve_geometry,
                preserve_edit_policy_raw,
                &rewire_components,
                &job.planned_components,
                &planned_components,
            ) {
                new_errors.push(serde_json::json!({
                    "kind": "preserve_mode_rejection",
                    "error": err,
                }));
            } else if !args.dry_run {
                match apply_plan_acceptance(
                    job,
                    draft,
                    preserve_existing_components,
                    planned_components,
                    notes,
                    defs,
                    rig_move_cycle_m,
                    plan_collider,
                    validated_reuse_groups,
                    reuse_warnings,
                ) {
                    Ok(()) => {
                        accepted = true;
                        still_pending = false;
                    }
                    Err(err) => {
                        new_errors.push(serde_json::json!({
                            "kind": "apply_error",
                            "error": err,
                        }));
                    }
                }
            } else {
                // dry_run: report whether acceptance would succeed under current policy gates.
                accepted = true;
                still_pending = false;
                let _ = (validated_reuse_groups, reuse_warnings, defs);
            }
        }
        Err(err) => {
            new_errors.push(serde_json::json!({
                "kind": "semantic_validation_error",
                "error": err,
            }));
        }
    }

    if !accepted {
        // Provide additional computed diagnostics (bounded) when still pending.
        let pending_for_inspect = Gen3dPendingPlanAttempt {
            call_id: pending_before.call_id.clone(),
            error: pending_before.error.clone(),
            preserve_existing_components: pending_before.preserve_existing_components,
            preserve_edit_policy: pending_before.preserve_edit_policy.clone(),
            rewire_components: pending_before.rewire_components.clone(),
            existing_component_names: pending_before.existing_component_names.clone(),
            existing_root_component: pending_before.existing_root_component.clone(),
            plan: plan_after.clone(),
        };
        let inspect = super::plan_tools::inspect_pending_plan_attempt_v1(
            Some(&pending_for_inspect),
            &job.planned_components,
            job.preserve_existing_components_mode,
        );
        if let Some(errors) = inspect
            .get("analysis")
            .and_then(|v| v.get("errors"))
            .and_then(|v| v.as_array())
        {
            for err in errors.iter().take(24) {
                new_errors.push(err.clone());
            }
        }
        if let Some(fixits) = inspect
            .get("analysis")
            .and_then(|v| v.get("fixits"))
            .and_then(|v| v.as_array())
        {
            for fixit in fixits.iter().take(8) {
                new_errors.push(serde_json::json!({
                    "kind": "fixit",
                    "fixit": fixit,
                }));
            }
        }

        if !args.dry_run {
            // Commit the patched plan into pending_plan_attempt for further inspection/patching.
            let mut pending_next = pending_for_inspect.clone();
            pending_next.plan = plan_after.clone();
            if let Some(first_error) = new_errors
                .iter()
                .find_map(|v| v.get("error").and_then(|e| e.as_str()))
            {
                pending_next.error = first_error.to_string();
            }
            job.pending_plan_attempt = Some(pending_next);
            committed = true;
        }
    } else if !args.dry_run {
        committed = true;
    }

    let assembly_rev_after = job.assembly_rev();

    let ok = accepted && rejected_ops.is_empty();

    let result = serde_json::json!({
        "ok": ok,
        "version": 1,
        "dry_run": args.dry_run,
        "base_plan": match base_plan { ApplyPlanOpsBasePlan::Pending => "pending", ApplyPlanOpsBasePlan::Current => "current" },
        "committed": committed,
        "accepted": accepted,
        "still_pending": still_pending,
        "assembly_rev_before": assembly_rev_before,
        "new_assembly_rev": assembly_rev_after,
        "applied_ops": applied_ops,
        "rejected_ops": rejected_ops,
        "diff_summary": diff_summary,
        "plan_before_after": {
            "before": summary_before,
            "after": summary_after,
        },
        "new_plan_summary": summary_after,
        "new_errors": if accepted { serde_json::Value::Null } else { serde_json::Value::Array(new_errors) },
    });

    if let Some(dir) = job.step_dir_path() {
        let log_ref = "plan_ops.jsonl";
        let ts_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);

        append_gen3d_jsonl_artifact(
            Some(dir),
            log_ref,
            &serde_json::json!({
                "ts_ms": ts_ms,
                "tool": "apply_plan_ops_v1",
                "call_id": call_id.unwrap_or(""),
                "active_workspace": job.active_workspace_id(),
                "assembly_rev_before": assembly_rev_before,
                "assembly_rev_after": assembly_rev_after,
                "dry_run": args.dry_run,
                "accepted": accepted,
                "result": result,
            }),
        );

        let filename = format!(
            "apply_plan_ops_last_{}.json",
            sanitize_prefix(call_id.unwrap_or(""))
        );
        super::artifacts::write_gen3d_json_artifact(Some(dir), filename, &result);
        super::artifacts::write_gen3d_json_artifact(Some(dir), "apply_plan_ops_last.json", &result);
    }

    Ok(result)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GeneratedPlanOpsJsonV1 {
    version: u32,
    ops: Vec<PlanOpJsonV1>,
}

fn apply_generated_plan_ops_micro_repairs_v1(
    payload: &mut serde_json::Value,
) -> Result<Vec<serde_json::Value>, String> {
    let Some(root) = payload.as_object_mut() else {
        return Ok(Vec::new());
    };
    let Some(ops) = root.get_mut("ops").and_then(|v| v.as_array_mut()) else {
        return Ok(Vec::new());
    };

    let mut diff: Vec<serde_json::Value> = Vec::new();

    for (idx, op) in ops.iter_mut().enumerate() {
        let Some(op_obj) = op.as_object_mut() else {
            continue;
        };
        let kind = op_obj
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if kind != "add_component" {
            continue;
        }

        let name = op_obj
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let component = op_obj
            .get("component")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        match (name.as_deref(), component.as_deref()) {
            (None, Some(component)) => {
                op_obj.insert(
                    "name".to_string(),
                    serde_json::Value::String(component.to_string()),
                );
                op_obj.remove("component");
                diff.push(serde_json::json!({
                    "kind": "alias_field",
                    "op_index": idx,
                    "op_kind": "add_component",
                    "from": "component",
                    "to": "name",
                    "value": component,
                }));
            }
            (Some(name), Some(component)) => {
                if name.trim() != component.trim() {
                    return Err(format!(
                        "llm_generate_plan_ops_v1: add_component op[{idx}] contains both `name` and `component` with different values (name={name:?}, component={component:?}); refusing deterministic micro-repair. Omit `component` and use only `name`."
                    ));
                }
                op_obj.remove("component");
                diff.push(serde_json::json!({
                    "kind": "dropped_redundant_alias",
                    "op_index": idx,
                    "op_kind": "add_component",
                    "dropped": "component",
                    "kept": "name",
                    "value": name,
                }));
            }
            _ => {}
        }
    }

    Ok(diff)
}

fn touched_component_names_for_scope(ops: &[PlanOpJsonV1]) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::<String>::new();
    for op in ops.iter() {
        match op {
            PlanOpJsonV1::AddComponent {
                name, attach_to, ..
            } => {
                let name = name.trim();
                if !name.is_empty() {
                    out.insert(name.to_string());
                }
                if let Some(att) = attach_to.as_ref() {
                    let parent = att.parent.trim();
                    if !parent.is_empty() {
                        out.insert(parent.to_string());
                    }
                }
            }
            PlanOpJsonV1::RemoveComponent { name } => {
                let name = name.trim();
                if !name.is_empty() {
                    out.insert(name.to_string());
                }
            }
            PlanOpJsonV1::SetAttachTo {
                component,
                set_attach_to,
            } => {
                let component = component.trim();
                if !component.is_empty() {
                    out.insert(component.to_string());
                }
                if let Some(att) = set_attach_to.as_ref() {
                    let parent = att.parent.trim();
                    if !parent.is_empty() {
                        out.insert(parent.to_string());
                    }
                }
            }
            PlanOpJsonV1::SetAnchor { component, .. } => {
                let component = component.trim();
                if !component.is_empty() {
                    out.insert(component.to_string());
                }
            }
            PlanOpJsonV1::SetAimComponents { components } => {
                for name in components.iter() {
                    let name = name.trim();
                    if !name.is_empty() {
                        out.insert(name.to_string());
                    }
                }
            }
            PlanOpJsonV1::SetAttackMuzzle { component, .. } => {
                let component = component.trim();
                if !component.is_empty() {
                    out.insert(component.to_string());
                }
            }
            PlanOpJsonV1::SetReuseGroups { reuse_groups } => {
                for group in reuse_groups.iter() {
                    let src = group.source.trim();
                    if !src.is_empty() {
                        out.insert(src.to_string());
                    }
                    for tgt in group.targets.iter() {
                        let tgt = tgt.trim();
                        if !tgt.is_empty() {
                            out.insert(tgt.to_string());
                        }
                    }
                }
            }
            PlanOpJsonV1::SetMobility { .. } | PlanOpJsonV1::SetCollider { .. } => {}
            PlanOpJsonV1::SetAttack { attack } => {
                let parsed: Result<AiAttackJson, _> = if attack.is_null() {
                    Err(())
                } else {
                    serde_json::from_value(attack.clone()).map_err(|_| ())
                };
                if let Ok(AiAttackJson::RangedProjectile { muzzle, .. }) = parsed {
                    if let Some(muzzle) = muzzle.as_ref() {
                        let component = muzzle.component.trim();
                        if !component.is_empty() {
                            out.insert(component.to_string());
                        }
                    }
                }
            }
        }
    }
    out
}

pub(super) fn apply_llm_generate_plan_ops_v1(
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    call_id: Option<&str>,
    preserve_existing_components: bool,
    preserve_edit_policy_raw: Option<&str>,
    rewire_components: Vec<String>,
    scope_components: Vec<String>,
    max_ops: usize,
    text: &str,
) -> Result<serde_json::Value, String> {
    if job.planned_components.is_empty() {
        return Err(
            "llm_generate_plan_ops_v1 requires an existing accepted plan. Run llm_generate_plan_v1 first."
                .into(),
        );
    }
    if !preserve_existing_components {
        return Err(
            "llm_generate_plan_ops_v1 requires preserve mode (constraints.preserve_existing_components=true)."
                .into(),
        );
    }

    let max_ops = max_ops.clamp(1, 64);

    let json_text = super::parse::extract_json_object(text).unwrap_or_else(|| text.to_string());
    let json_text = json_text.trim();
    let mut payload_value: serde_json::Value = serde_json::from_str(json_text)
        .map_err(|err| format!("llm_generate_plan_ops_v1: Failed to parse JSON: {err}"))?;
    let payload_value_for_artifact = payload_value.clone();

    let repair_diff = apply_generated_plan_ops_micro_repairs_v1(&mut payload_value)?;
    let repaired = !repair_diff.is_empty();
    let payload_value_normalized_for_artifact = payload_value.clone();

    let payload: GeneratedPlanOpsJsonV1 = serde_json::from_value(payload_value)
        .map_err(|err| format!("llm_generate_plan_ops_v1: AI JSON schema error: {err}"))?;
    if payload.version != 1 {
        return Err(format!(
            "llm_generate_plan_ops_v1: Unsupported version {} (expected 1)",
            payload.version
        ));
    }
    if payload.ops.len() > max_ops {
        return Err(format!(
            "llm_generate_plan_ops_v1: too many ops ({} > max_ops={max_ops}). Output fewer ops or increase max_ops (max 64).",
            payload.ops.len()
        ));
    }
    if payload.ops.len() > 64 {
        return Err("llm_generate_plan_ops_v1: too many ops (max 64)".into());
    }
    let ops_total = payload.ops.len();

    if let Some(dir) = job.step_dir_path() {
        let filename = format!(
            "plan_ops_generated_{}.json",
            sanitize_prefix(call_id.unwrap_or(""))
        );
        super::artifacts::write_gen3d_json_artifact(
            Some(dir),
            &filename,
            &payload_value_for_artifact,
        );
        super::artifacts::write_gen3d_json_artifact(
            Some(dir),
            "plan_ops_generated.json",
            &payload_value_for_artifact,
        );
        if repaired {
            let filename = format!(
                "plan_ops_generated_normalized_{}.json",
                sanitize_prefix(call_id.unwrap_or(""))
            );
            super::artifacts::write_gen3d_json_artifact(
                Some(dir),
                &filename,
                &payload_value_normalized_for_artifact,
            );
            super::artifacts::write_gen3d_json_artifact(
                Some(dir),
                "plan_ops_generated_normalized.json",
                &payload_value_normalized_for_artifact,
            );
        }
    }

    let scope_components: Vec<String> = scope_components
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let plan_json = super::plan_tools::build_preserve_mode_plan_template_json_v8(
        draft,
        &job.planned_components,
        &job.assembly_notes,
        job.plan_collider.as_ref(),
        job.rig_move_cycle_m,
        &job.reuse_groups,
    )?;
    let plan_before: AiPlanJsonV1 = serde_json::from_value(plan_json.clone()).map_err(|err| {
        format!("llm_generate_plan_ops_v1: failed to parse current plan snapshot: {err}")
    })?;

    let mut existing_component_names: Vec<String> = job
        .planned_components
        .iter()
        .map(|c| c.name.clone())
        .collect();
    existing_component_names.sort();
    existing_component_names.dedup();
    let existing_names_set: std::collections::HashSet<String> =
        existing_component_names.iter().cloned().collect();
    let existing_root_component = job
        .planned_components
        .iter()
        .find(|c| c.attach_to.is_none())
        .map(|c| c.name.clone());

    let touched = touched_component_names_for_scope(&payload.ops);
    let mut touched_existing: Vec<String> = touched
        .iter()
        .filter(|name| existing_names_set.contains(*name))
        .cloned()
        .collect();
    touched_existing.sort();
    touched_existing.dedup();

    let mut scope_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    for name in scope_components.iter() {
        scope_set.insert(name.clone());
    }
    let mut out_of_scope: Vec<String> = Vec::new();
    if !scope_components.is_empty() {
        for name in touched_existing.iter() {
            if !scope_set.contains(name) {
                out_of_scope.push(name.clone());
            }
        }
    }

    let assembly_rev_before = job.assembly_rev();
    let summary_before = plan_summary_json(&plan_before);

    if !out_of_scope.is_empty() {
        let err = format!(
            "llm_generate_plan_ops_v1 scope_components rejected ops touching out-of-scope existing components: {out_of_scope:?}\n\
Hint: include these names in scope_components, or omit scope_components, or use `llm_generate_plan_v1` for wide edits."
        );
        let result = serde_json::json!({
            "ok": false,
            "version": 1,
            "accepted": false,
            "committed": false,
            "assembly_rev_before": assembly_rev_before,
            "new_assembly_rev": job.assembly_rev(),
            "ops_total": ops_total,
            "diff_summary": serde_json::Value::Null,
            "new_plan_summary": summary_before,
            "new_errors": [{
                "kind": "scope_violation",
                "error": err.clone(),
                "scope_components_total": scope_components.len(),
                "scope_components_sample": scope_components.iter().cloned().take(32).collect::<Vec<_>>(),
                "touched_existing_components_total": touched_existing.len(),
                "touched_existing_components_sample": touched_existing.iter().cloned().take(32).collect::<Vec<_>>(),
                "out_of_scope_components": out_of_scope,
            }],
        });

        if let Some(dir) = job.step_dir_path() {
            let filename = format!(
                "plan_ops_apply_last_{}.json",
                sanitize_prefix(call_id.unwrap_or(""))
            );
            super::artifacts::write_gen3d_json_artifact(Some(dir), &filename, &result);
            super::artifacts::write_gen3d_json_artifact(
                Some(dir),
                "plan_ops_apply_last.json",
                &result,
            );
            super::artifacts::write_gen3d_json_artifact(
                Some(dir),
                "plan_ops_plan_before.json",
                &plan_json,
            );
        }
        return Ok(result);
    }

    let plan_before_serialized = plan_json;

    let mut plan_after = plan_before.clone();
    let ops = payload.ops;
    let ops_total = ops.len();
    let (applied_ops, rejected_ops, state) = apply_ops_inner(ops, &mut plan_after);

    let summary_after = plan_summary_json(&plan_after);
    let diff_summary = serde_json::json!({
        "components": {
            "added": state.components_added,
            "removed": state.components_removed,
        },
        "attachments_set": state.attachments_set,
        "anchors_upserted": state.anchors_upserted,
        "aim_set": state.aim_set,
        "mobility_set": state.mobility_set,
        "attack_set": state.attack_set,
        "attack_muzzle_set": state.muzzle_set,
        "collider_set": state.collider_set,
        "reuse_groups_set": state.reuse_groups_set,
        "touched_components": state.touched_components.iter().cloned().take(32).collect::<Vec<_>>(),
    });
    if !rejected_ops.is_empty() {
        let mut lines: Vec<String> = Vec::new();
        lines.push(format!(
            "llm_generate_plan_ops_v1 rejected {} op(s); refusing partial apply (atomic mode).",
            rejected_ops.len()
        ));
        for rej in rejected_ops.iter().take(12) {
            lines.push(format!(
                "- op[{}] kind={} error={}",
                rej.index, rej.kind, rej.error
            ));
        }
        if rejected_ops.len() > 12 {
            lines.push(format!(
                "- … ({} more)",
                rejected_ops.len().saturating_sub(12)
            ));
        }
        lines.push(
            "Hint: Fix rejected ops and retry. This tool commits only when ALL ops are accepted."
                .into(),
        );
        let err = lines.join("\n");

        let result = serde_json::json!({
            "ok": false,
            "version": 1,
            "repaired": repaired,
            "repair_diff": repair_diff,
            "accepted": false,
            "committed": false,
            "assembly_rev_before": assembly_rev_before,
            "new_assembly_rev": job.assembly_rev(),
            "ops_total": ops_total,
            "applied_ops": applied_ops,
            "rejected_ops": rejected_ops,
            "diff_summary": diff_summary,
            "plan_before_after": {
                "before": summary_before,
                "after": summary_after,
            },
            "new_plan_summary": summary_after,
            "new_errors": [{
                "kind": "op_rejected",
                "error": err.clone(),
            }],
        });

        if let Some(dir) = job.step_dir_path() {
            let filename = format!(
                "plan_ops_apply_last_{}.json",
                sanitize_prefix(call_id.unwrap_or(""))
            );
            super::artifacts::write_gen3d_json_artifact(Some(dir), &filename, &result);
            super::artifacts::write_gen3d_json_artifact(
                Some(dir),
                "plan_ops_apply_last.json",
                &result,
            );
            super::artifacts::write_gen3d_json_artifact(
                Some(dir),
                "plan_ops_plan_before.json",
                &plan_before_serialized,
            );
        }

        return Ok(result);
    }

    let can_preserve_geometry = preserve_existing_components
        && !job.planned_components.is_empty()
        && draft.total_non_projectile_primitive_parts() > 0;

    let mut accepted = false;
    let mut new_errors: Vec<serde_json::Value> = Vec::new();

    match convert::ai_plan_to_initial_draft_defs(plan_after.clone()) {
        Ok((planned_components, notes, defs)) => {
            let rig_move_cycle_m = plan_after
                .rig
                .as_ref()
                .and_then(|r| r.move_cycle_m)
                .filter(|v| v.is_finite())
                .map(f32::abs)
                .filter(|v| *v > 1e-3);
            let plan_collider = plan_after.collider.clone();
            let (validated_reuse_groups, reuse_warnings) =
                reuse_groups::validate_reuse_groups(&plan_after.reuse_groups, &planned_components);

            if let Some(err) = preserve_error_for_plan_apply(
                "llm_generate_plan_ops_v1",
                can_preserve_geometry,
                preserve_edit_policy_raw,
                &rewire_components,
                &job.planned_components,
                &planned_components,
            ) {
                new_errors.push(serde_json::json!({
                    "kind": "preserve_mode_rejection",
                    "error": err,
                }));
            } else {
                match apply_plan_acceptance(
                    job,
                    draft,
                    preserve_existing_components,
                    planned_components,
                    notes,
                    defs,
                    rig_move_cycle_m,
                    plan_collider,
                    validated_reuse_groups,
                    reuse_warnings,
                ) {
                    Ok(()) => {
                        accepted = true;
                    }
                    Err(err) => {
                        new_errors.push(serde_json::json!({
                            "kind": "apply_error",
                            "error": err,
                        }));
                    }
                }
            }
        }
        Err(err) => {
            new_errors.push(serde_json::json!({
                "kind": "semantic_validation_error",
                "error": err,
            }));
        }
    }

    if !accepted {
        let pending_for_inspect = Gen3dPendingPlanAttempt {
            call_id: call_id.unwrap_or("llm_generate_plan_ops_v1").to_string(),
            error: new_errors
                .iter()
                .find_map(|v| v.get("error").and_then(|e| e.as_str()))
                .unwrap_or("plan ops apply rejected")
                .to_string(),
            preserve_existing_components,
            preserve_edit_policy: preserve_edit_policy_raw
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            rewire_components: rewire_components.clone(),
            existing_component_names: existing_component_names.clone(),
            existing_root_component: existing_root_component.clone(),
            plan: plan_after.clone(),
        };

        let inspect = super::plan_tools::inspect_pending_plan_attempt_v1(
            Some(&pending_for_inspect),
            &job.planned_components,
            job.preserve_existing_components_mode,
        );
        if let Some(errors) = inspect
            .get("analysis")
            .and_then(|v| v.get("errors"))
            .and_then(|v| v.as_array())
        {
            for err in errors.iter().take(24) {
                new_errors.push(err.clone());
            }
        }
        if let Some(fixits) = inspect
            .get("analysis")
            .and_then(|v| v.get("fixits"))
            .and_then(|v| v.as_array())
        {
            for fixit in fixits.iter().take(8) {
                new_errors.push(serde_json::json!({
                    "kind": "fixit",
                    "fixit": fixit,
                }));
            }
        }

        job.pending_plan_attempt = Some(pending_for_inspect);
    }
    let committed = accepted;

    let assembly_rev_after = job.assembly_rev();
    let ok = accepted && rejected_ops.is_empty();
    let result = serde_json::json!({
        "ok": ok,
        "version": 1,
        "repaired": repaired,
        "repair_diff": repair_diff,
        "accepted": accepted,
        "committed": committed,
        "assembly_rev_before": assembly_rev_before,
        "new_assembly_rev": assembly_rev_after,
        "ops_total": ops_total,
        "applied_ops": applied_ops,
        "rejected_ops": rejected_ops,
        "diff_summary": diff_summary,
        "plan_before_after": {
            "before": summary_before,
            "after": summary_after,
        },
        "new_plan_summary": summary_after,
        "new_errors": if accepted { serde_json::Value::Null } else { serde_json::Value::Array(new_errors) },
    });

    if let Some(dir) = job.step_dir_path() {
        let filename = format!(
            "plan_ops_apply_last_{}.json",
            sanitize_prefix(call_id.unwrap_or(""))
        );
        super::artifacts::write_gen3d_json_artifact(Some(dir), &filename, &result);
        super::artifacts::write_gen3d_json_artifact(Some(dir), "plan_ops_apply_last.json", &result);
        super::artifacts::write_gen3d_json_artifact(
            Some(dir),
            "plan_ops_plan_before.json",
            &plan_before_serialized,
        );
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen3d::ai::schema::AiMobilityJson;
    use crate::gen3d::ai::schema::{AiJointJson, AiJointKindJson};
    use crate::object::registry::{
        ColliderProfile, MeshKey, MobilityMode, ObjectPartDef, PrimitiveVisualDef, UnitAttackKind,
    };
    use uuid::Uuid;

    fn make_plan_with_missing_parent() -> AiPlanJsonV1 {
        AiPlanJsonV1 {
            version: 8,
            rig: None,
            mobility: AiMobilityJson::Static,
            attack: None,
            aim: None,
            collider: None,
            assembly_notes: String::new(),
            root_component: None,
            reuse_groups: Vec::new(),
            components: vec![
                AiPlanComponentJson {
                    name: "body".into(),
                    purpose: String::new(),
                    modeling_notes: String::new(),
                    size: [1.0, 1.0, 1.0],
                    anchors: Vec::new(),
                    contacts: Vec::new(),
                    articulation_nodes: Vec::new(),
                    attach_to: None,
                },
                AiPlanComponentJson {
                    name: "gun".into(),
                    purpose: String::new(),
                    modeling_notes: String::new(),
                    size: [0.2, 0.2, 0.6],
                    anchors: Vec::new(),
                    contacts: Vec::new(),
                    articulation_nodes: Vec::new(),
                    attach_to: Some(AiPlanAttachmentJson {
                        parent: "arm_lower_r".into(),
                        parent_anchor: "origin".into(),
                        child_anchor: "origin".into(),
                        offset: None,
                        joint: None,
                    }),
                },
            ],
        }
    }

    #[test]
    fn apply_plan_ops_accepts_after_fixing_missing_component() {
        let mut job = Gen3dAiJob::default();
        let mut draft = Gen3dDraft { defs: Vec::new() };

        job.pending_plan_attempt = Some(Gen3dPendingPlanAttempt {
            call_id: "call_1".into(),
            error: "unknown parent".into(),
            preserve_existing_components: false,
            preserve_edit_policy: None,
            rewire_components: Vec::new(),
            existing_component_names: Vec::new(),
            existing_root_component: None,
            plan: make_plan_with_missing_parent(),
        });

        let result = apply_plan_ops_v1(
            &mut job,
            &mut draft,
            Some("call_apply"),
            serde_json::json!({
                "version": 1,
                "ops": [
                    { "kind": "add_component", "name": "arm_lower_r", "size": [0.3, 0.2, 0.2] },
                    {
                        "kind": "set_attach_to",
                        "component": "arm_lower_r",
                        "set_attach_to": { "parent": "body", "parent_anchor": "origin", "child_anchor": "origin" }
                    }
                ]
            }),
        )
        .expect("tool should return result JSON");

        assert!(
            result
                .get("accepted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            "expected accepted=true, got {result:?}"
        );
        assert!(!result
            .get("still_pending")
            .and_then(|v| v.as_bool())
            .unwrap_or(true));
        assert!(job.pending_plan_attempt.is_none());
        assert_eq!(job.planned_components.len(), 3);
    }

    #[test]
    fn apply_plan_ops_rejects_remove_component_when_referenced() {
        let mut job = Gen3dAiJob::default();
        let mut draft = Gen3dDraft { defs: Vec::new() };

        let mut plan = make_plan_with_missing_parent();
        plan.components.push(AiPlanComponentJson {
            name: "neck".into(),
            purpose: String::new(),
            modeling_notes: String::new(),
            size: [0.2, 0.2, 0.2],
            anchors: Vec::new(),
            contacts: Vec::new(),
            articulation_nodes: Vec::new(),
            attach_to: Some(AiPlanAttachmentJson {
                parent: "body".into(),
                parent_anchor: "origin".into(),
                child_anchor: "origin".into(),
                offset: None,
                joint: None,
            }),
        });

        job.pending_plan_attempt = Some(Gen3dPendingPlanAttempt {
            call_id: "call_2".into(),
            error: "unknown parent".into(),
            preserve_existing_components: false,
            preserve_edit_policy: None,
            rewire_components: Vec::new(),
            existing_component_names: Vec::new(),
            existing_root_component: None,
            plan,
        });

        let result = apply_plan_ops_v1(
            &mut job,
            &mut draft,
            Some("call_apply"),
            serde_json::json!({
                "version": 1,
                "ops": [
                    { "kind": "remove_component", "name": "body" }
                ]
            }),
        )
        .expect("tool should return result JSON");

        assert!(!result
            .get("accepted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false));
        assert!(result
            .get("still_pending")
            .and_then(|v| v.as_bool())
            .unwrap_or(false));
        let rejected = result
            .get("rejected_ops")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert_eq!(rejected.len(), 1);
        let error = rejected[0]
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(error.contains("referenced"));
    }

    fn make_simple_current_plan() -> AiPlanJsonV1 {
        AiPlanJsonV1 {
            version: 8,
            rig: None,
            mobility: AiMobilityJson::Static,
            attack: None,
            aim: None,
            collider: None,
            assembly_notes: "current".into(),
            root_component: None,
            reuse_groups: Vec::new(),
            components: vec![
                AiPlanComponentJson {
                    name: "torso".into(),
                    purpose: String::new(),
                    modeling_notes: String::new(),
                    size: [1.0, 1.0, 1.0],
                    anchors: Vec::new(),
                    contacts: Vec::new(),
                    articulation_nodes: Vec::new(),
                    attach_to: None,
                },
                AiPlanComponentJson {
                    name: "head".into(),
                    purpose: String::new(),
                    modeling_notes: String::new(),
                    size: [0.5, 0.5, 0.5],
                    anchors: Vec::new(),
                    contacts: Vec::new(),
                    articulation_nodes: Vec::new(),
                    attach_to: Some(AiPlanAttachmentJson {
                        parent: "torso".into(),
                        parent_anchor: "origin".into(),
                        child_anchor: "origin".into(),
                        offset: None,
                        joint: None,
                    }),
                },
            ],
        }
    }

    #[test]
    fn apply_plan_ops_can_patch_current_accepted_plan() {
        let mut job = Gen3dAiJob::default();
        let mut draft = Gen3dDraft { defs: Vec::new() };

        let plan = make_simple_current_plan();
        let (planned, notes, mut defs) =
            convert::ai_plan_to_initial_draft_defs(plan).expect("plan should convert");
        job.planned_components = planned;
        job.assembly_notes = notes;
        job.preserve_existing_components_mode = true;

        // Mark the draft as already-generated so preserve-mode validation/merge logic runs.
        let torso_id =
            crate::object::registry::builtin_object_id("gravimera/gen3d/component/torso");
        let torso_def = defs
            .iter_mut()
            .find(|def| def.object_id == torso_id)
            .expect("expected torso def");
        torso_def.parts.push(
            ObjectPartDef::primitive(
                PrimitiveVisualDef::Primitive {
                    mesh: MeshKey::UnitCube,
                    params: None,
                    color: Color::srgb(0.5, 0.5, 0.5),
                    unlit: false,
                },
                Transform::IDENTITY,
            )
            .with_part_id(Uuid::new_v4().as_u128()),
        );
        draft.defs = defs;

        let result = apply_plan_ops_v1(
            &mut job,
            &mut draft,
            Some("call_apply"),
            serde_json::json!({
                "version": 1,
                "base_plan": "current",
                "ops": [
                    { "kind": "add_component", "name": "hat", "size": [0.3, 0.2, 0.3] },
                    { "kind": "set_attach_to", "component": "hat", "set_attach_to": { "parent": "head", "parent_anchor": "origin", "child_anchor": "origin" } }
                ]
            }),
        )
        .expect("tool should return result JSON");

        assert!(
            result
                .get("accepted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            "expected accepted=true, got {result:?}"
        );
        assert!(job.planned_components.iter().any(|c| c.name == "hat"));
        assert_eq!(job.planned_components.len(), 3);
    }

    #[test]
    fn apply_plan_ops_can_set_root_fields_on_current_plan() {
        let mut job = Gen3dAiJob::default();
        let mut draft = Gen3dDraft { defs: Vec::new() };

        let plan = make_simple_current_plan();
        let (planned, notes, mut defs) =
            convert::ai_plan_to_initial_draft_defs(plan).expect("plan should convert");
        job.planned_components = planned;
        job.assembly_notes = notes;
        job.preserve_existing_components_mode = true;

        // Mark the draft as already-generated so preserve-mode validation/merge logic runs.
        let torso_id =
            crate::object::registry::builtin_object_id("gravimera/gen3d/component/torso");
        let torso_def = defs
            .iter_mut()
            .find(|def| def.object_id == torso_id)
            .expect("expected torso def");
        torso_def.parts.push(
            ObjectPartDef::primitive(
                PrimitiveVisualDef::Primitive {
                    mesh: MeshKey::UnitCube,
                    params: None,
                    color: Color::srgb(0.5, 0.5, 0.5),
                    unlit: false,
                },
                Transform::IDENTITY,
            )
            .with_part_id(Uuid::new_v4().as_u128()),
        );
        draft.defs = defs;

        let result = apply_plan_ops_v1(
            &mut job,
            &mut draft,
            Some("call_apply"),
            serde_json::json!({
                "version": 1,
                "base_plan": "current",
                "ops": [
                    { "kind": "set_mobility", "mobility": { "kind": "ground", "max_speed": 5.0 } },
                    { "kind": "set_collider", "collider": { "kind": "circle_xz", "radius": 1.2 } },
                    { "kind": "set_attack", "attack": { "kind": "melee", "damage": 10, "range": 1.0, "radius": 0.5, "arc_degrees": 90.0 } }
                ]
            }),
        )
        .expect("tool should return result JSON");

        assert!(
            result
                .get("accepted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            "expected accepted=true, got {result:?}"
        );
        assert_eq!(
            result
                .get("diff_summary")
                .and_then(|v| v.get("mobility_set"))
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            result
                .get("diff_summary")
                .and_then(|v| v.get("attack_set"))
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            result
                .get("diff_summary")
                .and_then(|v| v.get("collider_set"))
                .and_then(|v| v.as_bool()),
            Some(true)
        );

        let root_def = draft.root_def().expect("expected root def");
        assert!(
            root_def
                .mobility
                .as_ref()
                .is_some_and(|m| m.mode == MobilityMode::Ground && (m.max_speed - 5.0).abs() < 1e-4),
            "expected ground mobility, got {:?}",
            root_def.mobility
        );
        assert!(
            root_def
                .attack
                .as_ref()
                .is_some_and(|attack| attack.kind == UnitAttackKind::Melee),
            "expected melee attack, got {:?}",
            root_def.attack.as_ref().map(|a| a.kind)
        );
        assert!(
            matches!(root_def.collider, ColliderProfile::CircleXZ { radius } if (radius - 1.2).abs() < 1e-4),
            "expected circle collider, got {:?}",
            root_def.collider
        );
    }

    #[test]
    fn apply_plan_ops_set_attack_and_collider_accepts_null_when_static() {
        let mut job = Gen3dAiJob::default();
        let mut draft = Gen3dDraft { defs: Vec::new() };

        let plan = make_simple_current_plan();
        let (planned, notes, mut defs) =
            convert::ai_plan_to_initial_draft_defs(plan).expect("plan should convert");
        job.planned_components = planned;
        job.assembly_notes = notes;
        job.preserve_existing_components_mode = true;

        // Mark the draft as already-generated so preserve-mode validation/merge logic runs.
        let torso_id =
            crate::object::registry::builtin_object_id("gravimera/gen3d/component/torso");
        let torso_def = defs
            .iter_mut()
            .find(|def| def.object_id == torso_id)
            .expect("expected torso def");
        torso_def.parts.push(
            ObjectPartDef::primitive(
                PrimitiveVisualDef::Primitive {
                    mesh: MeshKey::UnitCube,
                    params: None,
                    color: Color::srgb(0.5, 0.5, 0.5),
                    unlit: false,
                },
                Transform::IDENTITY,
            )
            .with_part_id(Uuid::new_v4().as_u128()),
        );
        draft.defs = defs;

        let _ = apply_plan_ops_v1(
            &mut job,
            &mut draft,
            Some("call_apply_1"),
            serde_json::json!({
                "version": 1,
                "base_plan": "current",
                "ops": [
                    { "kind": "set_mobility", "mobility": { "kind": "ground", "max_speed": 5.0 } },
                    { "kind": "set_collider", "collider": { "kind": "circle_xz", "radius": 1.2 } },
                    { "kind": "set_attack", "attack": { "kind": "melee", "damage": 10, "range": 1.0, "radius": 0.5, "arc_degrees": 90.0 } }
                ]
            }),
        )
        .expect("tool should return result JSON");

        let result = apply_plan_ops_v1(
            &mut job,
            &mut draft,
            Some("call_apply_2"),
            serde_json::json!({
                "version": 1,
                "base_plan": "current",
                "ops": [
                    { "kind": "set_mobility", "mobility": { "kind": "static" } },
                    { "kind": "set_attack", "attack": null },
                    { "kind": "set_collider", "collider": null }
                ]
            }),
        )
        .expect("tool should return result JSON");

        assert!(
            result
                .get("accepted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            "expected accepted=true, got {result:?}"
        );

        let root_def = draft.root_def().expect("expected root def");
        assert!(
            root_def.mobility.is_none(),
            "expected mobility removed, got {:?}",
            root_def.mobility
        );
        assert!(
            root_def.attack.is_none(),
            "expected attack removed, got {:?}",
            root_def.attack.as_ref().map(|a| a.kind)
        );
        assert!(
            matches!(root_def.collider, ColliderProfile::AabbXZ { .. }),
            "expected default aabb collider, got {:?}",
            root_def.collider
        );
        assert!(
            job.plan_collider.is_none(),
            "expected plan_collider=None, got {:?}",
            job.plan_collider
        );
    }

    #[test]
    fn apply_plan_ops_current_failure_creates_pending_attempt() {
        let mut job = Gen3dAiJob::default();
        let mut draft = Gen3dDraft { defs: Vec::new() };

        let plan = make_simple_current_plan();
        let (planned, notes, defs) =
            convert::ai_plan_to_initial_draft_defs(plan).expect("plan should convert");
        job.planned_components = planned;
        job.assembly_notes = notes;
        draft.defs = defs;

        assert!(job.pending_plan_attempt.is_none());

        let result = apply_plan_ops_v1(
            &mut job,
            &mut draft,
            Some("call_apply"),
            serde_json::json!({
                "version": 1,
                "base_plan": "current",
                "ops": [
                    {
                        "kind": "add_component",
                        "name": "bad",
                        "size": [0.2, 0.2, 0.2],
                        "attach_to": { "parent": "nope", "parent_anchor": "origin", "child_anchor": "origin" }
                    }
                ]
            }),
        )
        .expect("tool should return result JSON");

        assert!(!result
            .get("accepted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false));
        assert!(job.pending_plan_attempt.is_some());
        let pending = job
            .pending_plan_attempt
            .as_ref()
            .expect("expected pending attempt");
        assert!(
            pending.error.contains("attach_to parent"),
            "unexpected error: {}",
            pending.error
        );
        assert!(pending.plan.components.iter().any(|c| c.name == "bad"));
    }

    #[test]
    fn llm_generate_plan_ops_scope_rejects_out_of_scope_existing_components() {
        let mut job = Gen3dAiJob::default();
        let mut draft = Gen3dDraft { defs: Vec::new() };

        let plan = make_simple_current_plan();
        let (planned, notes, defs) =
            convert::ai_plan_to_initial_draft_defs(plan).expect("plan should convert");
        job.planned_components = planned;
        job.assembly_notes = notes;
        job.preserve_existing_components_mode = true;
        draft.defs = defs;

        assert!(job.pending_plan_attempt.is_none());

        let text = r#"
        {
          "version": 1,
          "ops": [
            { "kind": "set_attach_to", "component": "torso", "set_attach_to": null }
          ]
        }
        "#;

        let result = apply_llm_generate_plan_ops_v1(
            &mut job,
            &mut draft,
            Some("call_plan_ops"),
            true,
            Some("additive"),
            Vec::new(),
            vec!["head".into()],
            32,
            text,
        )
        .expect("tool should return result JSON");

        assert!(!result
            .get("accepted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false));
        assert!(!result
            .get("committed")
            .and_then(|v| v.as_bool())
            .unwrap_or(true));
        assert!(job.pending_plan_attempt.is_none());

        let err = result
            .get("new_errors")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.get("error"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(err.contains("out-of-scope"), "{err}");
        assert!(err.contains("scope_components"), "{err}");
    }

    fn make_three_component_chain_plan() -> AiPlanJsonV1 {
        AiPlanJsonV1 {
            version: 8,
            rig: None,
            mobility: AiMobilityJson::Static,
            attack: None,
            aim: None,
            collider: None,
            assembly_notes: "test".into(),
            root_component: Some("torso".into()),
            reuse_groups: Vec::new(),
            components: vec![
                AiPlanComponentJson {
                    name: "torso".into(),
                    purpose: String::new(),
                    modeling_notes: String::new(),
                    size: [1.0, 1.0, 1.0],
                    anchors: Vec::new(),
                    contacts: Vec::new(),
                    articulation_nodes: Vec::new(),
                    attach_to: None,
                },
                AiPlanComponentJson {
                    name: "neck".into(),
                    purpose: String::new(),
                    modeling_notes: String::new(),
                    size: [0.2, 0.2, 0.2],
                    anchors: Vec::new(),
                    contacts: Vec::new(),
                    articulation_nodes: Vec::new(),
                    attach_to: Some(AiPlanAttachmentJson {
                        parent: "torso".into(),
                        parent_anchor: "origin".into(),
                        child_anchor: "origin".into(),
                        offset: None,
                        joint: None,
                    }),
                },
                AiPlanComponentJson {
                    name: "head".into(),
                    purpose: String::new(),
                    modeling_notes: String::new(),
                    size: [0.5, 0.5, 0.5],
                    anchors: Vec::new(),
                    contacts: Vec::new(),
                    articulation_nodes: Vec::new(),
                    attach_to: Some(AiPlanAttachmentJson {
                        parent: "neck".into(),
                        parent_anchor: "origin".into(),
                        child_anchor: "origin".into(),
                        offset: None,
                        joint: None,
                    }),
                },
            ],
        }
    }

    #[test]
    fn llm_generate_plan_ops_enforces_preserve_policy_for_disallowed_rewire() {
        let mut job = Gen3dAiJob::default();
        let mut draft = Gen3dDraft { defs: Vec::new() };

        let plan = make_three_component_chain_plan();
        let (planned, notes, mut defs) =
            convert::ai_plan_to_initial_draft_defs(plan).expect("plan should convert");
        job.planned_components = planned;
        job.assembly_notes = notes;
        job.preserve_existing_components_mode = true;

        // Mark the draft as already-generated so preserve-mode validation runs.
        let torso_id =
            crate::object::registry::builtin_object_id("gravimera/gen3d/component/torso");
        let torso_def = defs
            .iter_mut()
            .find(|def| def.object_id == torso_id)
            .expect("expected torso def");
        torso_def.parts.push(
            ObjectPartDef::primitive(
                PrimitiveVisualDef::Primitive {
                    mesh: MeshKey::UnitCube,
                    params: None,
                    color: Color::srgb(0.5, 0.5, 0.5),
                    unlit: false,
                },
                Transform::IDENTITY,
            )
            .with_part_id(Uuid::new_v4().as_u128()),
        );
        draft.defs = defs;

        let text = r#"
        {
          "version": 1,
          "ops": [
            {
              "kind": "set_attach_to",
              "component": "head",
              "set_attach_to": { "parent": "torso", "parent_anchor": "origin", "child_anchor": "origin" }
            }
          ]
        }
        "#;

        let result = apply_llm_generate_plan_ops_v1(
            &mut job,
            &mut draft,
            Some("call_plan_ops"),
            true,
            Some("additive"),
            Vec::new(),
            Vec::new(),
            32,
            text,
        )
        .expect("tool should return result JSON");

        assert!(!result
            .get("accepted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false));
        assert!(job.pending_plan_attempt.is_some());

        let errors = result
            .get("new_errors")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let preserve_err = errors
            .iter()
            .find(|e| e.get("kind").and_then(|v| v.as_str()) == Some("preserve_mode_rejection"))
            .and_then(|e| e.get("error").and_then(|v| v.as_str()))
            .unwrap_or("");
        assert!(
            preserve_err.contains("edit_policy=additive rejected plan diff"),
            "{preserve_err}"
        );
        assert!(preserve_err.contains("component=head"), "{preserve_err}");
        assert!(
            preserve_err.contains("attach_to.(parent,parent_anchor,child_anchor)"),
            "{preserve_err}"
        );
    }

    #[test]
    fn llm_generate_plan_ops_preserves_notes_on_unrelated_patch() {
        let mut job = Gen3dAiJob::default();
        let mut draft = Gen3dDraft { defs: Vec::new() };

        let plan = make_simple_current_plan();
        let (mut planned, _notes, mut defs) =
            convert::ai_plan_to_initial_draft_defs(plan).expect("plan should convert");

        planned[0].modeling_notes = "torso modeling notes keep".into();
        planned[1].modeling_notes = "head modeling notes keep".into();

        job.planned_components = planned;
        job.assembly_notes = "assembly notes keep".into();
        job.preserve_existing_components_mode = true;

        // Mark the draft as already-generated so preserve-mode merge path is used.
        let torso_id =
            crate::object::registry::builtin_object_id("gravimera/gen3d/component/torso");
        let torso_def = defs
            .iter_mut()
            .find(|def| def.object_id == torso_id)
            .expect("expected torso def");
        torso_def.parts.push(
            ObjectPartDef::primitive(
                PrimitiveVisualDef::Primitive {
                    mesh: MeshKey::UnitCube,
                    params: None,
                    color: Color::srgb(0.5, 0.5, 0.5),
                    unlit: false,
                },
                Transform::IDENTITY,
            )
            .with_part_id(Uuid::new_v4().as_u128()),
        );
        draft.defs = defs;

        let text = r#"
        {
          "version": 1,
          "ops": [
            { "kind": "add_component", "name": "hat", "size": [0.3, 0.2, 0.3] },
            {
              "kind": "set_attach_to",
              "component": "hat",
              "set_attach_to": { "parent": "head", "parent_anchor": "origin", "child_anchor": "origin" }
            }
          ]
        }
        "#;

        let result = apply_llm_generate_plan_ops_v1(
            &mut job,
            &mut draft,
            Some("call_plan_ops"),
            true,
            Some("additive"),
            Vec::new(),
            Vec::new(),
            32,
            text,
        )
        .expect("tool should return result JSON");

        assert!(result
            .get("accepted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false));
        assert_eq!(job.assembly_notes.trim(), "assembly notes keep");
        let torso = job
            .planned_components
            .iter()
            .find(|c| c.name == "torso")
            .expect("expected torso");
        assert_eq!(torso.modeling_notes.trim(), "torso modeling notes keep");
        assert!(job.planned_components.iter().any(|c| c.name == "hat"));
    }

    #[test]
    fn llm_generate_plan_ops_micro_repairs_add_component_component_alias() {
        let mut job = Gen3dAiJob::default();
        let mut draft = Gen3dDraft { defs: Vec::new() };

        let plan = make_simple_current_plan();
        let (planned, notes, mut defs) =
            convert::ai_plan_to_initial_draft_defs(plan).expect("plan should convert");

        job.planned_components = planned;
        job.assembly_notes = notes;
        job.preserve_existing_components_mode = true;

        // Mark the draft as already-generated so preserve-mode merge path is used.
        let torso_id =
            crate::object::registry::builtin_object_id("gravimera/gen3d/component/torso");
        let torso_def = defs
            .iter_mut()
            .find(|def| def.object_id == torso_id)
            .expect("expected torso def");
        torso_def.parts.push(
            ObjectPartDef::primitive(
                PrimitiveVisualDef::Primitive {
                    mesh: MeshKey::UnitCube,
                    params: None,
                    color: Color::srgb(0.5, 0.5, 0.5),
                    unlit: false,
                },
                Transform::IDENTITY,
            )
            .with_part_id(Uuid::new_v4().as_u128()),
        );
        draft.defs = defs;

        let text = r#"
        {
          "version": 1,
          "ops": [
            { "kind": "add_component", "component": "hat", "size": [0.3, 0.2, 0.3] },
            {
              "kind": "set_attach_to",
              "component": "hat",
              "set_attach_to": { "parent": "head", "parent_anchor": "origin", "child_anchor": "origin" }
            }
          ]
        }
        "#;

        let result = apply_llm_generate_plan_ops_v1(
            &mut job,
            &mut draft,
            Some("call_plan_ops"),
            true,
            Some("additive"),
            Vec::new(),
            Vec::new(),
            32,
            text,
        )
        .expect("tool should return result JSON");

        assert!(result
            .get("accepted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false));
        assert_eq!(result.get("repaired").and_then(|v| v.as_bool()), Some(true));
        let diff = result
            .get("repair_diff")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(!diff.is_empty());
        assert_eq!(
            diff.iter().any(
                |v| v.get("from").and_then(|v| v.as_str()) == Some("component")
                    && v.get("to").and_then(|v| v.as_str()) == Some("name")
            ),
            true
        );
        assert!(job.planned_components.iter().any(|c| c.name == "hat"));
    }

    fn make_plan_with_tail_joint(joint: Option<AiJointJson>) -> AiPlanJsonV1 {
        AiPlanJsonV1 {
            version: 8,
            rig: None,
            mobility: AiMobilityJson::Static,
            attack: None,
            aim: None,
            collider: None,
            assembly_notes: "test".into(),
            root_component: None,
            reuse_groups: Vec::new(),
            components: vec![
                AiPlanComponentJson {
                    name: "body".into(),
                    purpose: String::new(),
                    modeling_notes: String::new(),
                    size: [1.0, 1.0, 1.0],
                    anchors: Vec::new(),
                    contacts: Vec::new(),
                    articulation_nodes: Vec::new(),
                    attach_to: None,
                },
                AiPlanComponentJson {
                    name: "tail".into(),
                    purpose: String::new(),
                    modeling_notes: String::new(),
                    size: [0.2, 0.2, 0.6],
                    anchors: Vec::new(),
                    contacts: Vec::new(),
                    articulation_nodes: Vec::new(),
                    attach_to: Some(AiPlanAttachmentJson {
                        parent: "body".into(),
                        parent_anchor: "origin".into(),
                        child_anchor: "origin".into(),
                        offset: None,
                        joint,
                    }),
                },
            ],
        }
    }

    #[test]
    fn preserve_mode_acceptance_preserves_attachment_joint_when_interface_unchanged() {
        let hinge = AiJointJson {
            kind: AiJointKindJson::Hinge,
            axis_join: Some([0.0, 1.0, 0.0]),
            limits_degrees: Some([-45.0, 45.0]),
            swing_limits_degrees: None,
            twist_limits_degrees: None,
        };

        let mut job = Gen3dAiJob::default();
        let mut draft = Gen3dDraft { defs: Vec::new() };

        let plan_with_joint = make_plan_with_tail_joint(Some(hinge.clone()));
        let (planned_old, notes_old, mut defs_old) =
            convert::ai_plan_to_initial_draft_defs(plan_with_joint).expect("plan should convert");
        job.planned_components = planned_old;
        job.assembly_notes = notes_old;

        // Mark the draft as already-generated so preserve-mode acceptance merges metadata.
        let body_id = crate::object::registry::builtin_object_id("gravimera/gen3d/component/body");
        let body_def = defs_old
            .iter_mut()
            .find(|def| def.object_id == body_id)
            .expect("expected body def");
        body_def.parts.push(
            ObjectPartDef::primitive(
                PrimitiveVisualDef::Primitive {
                    mesh: MeshKey::UnitCube,
                    params: None,
                    color: Color::srgb(0.5, 0.5, 0.5),
                    unlit: false,
                },
                Transform::IDENTITY,
            )
            .with_part_id(Uuid::new_v4().as_u128()),
        );
        draft.defs = defs_old;

        let plan_without_joint = make_plan_with_tail_joint(None);
        let (planned_new, notes_new, defs_new) =
            convert::ai_plan_to_initial_draft_defs(plan_without_joint)
                .expect("plan should convert");

        apply_plan_acceptance(
            &mut job,
            &mut draft,
            true,
            planned_new,
            notes_new,
            defs_new,
            None,
            None,
            Vec::new(),
            Vec::new(),
        )
        .expect("acceptance should succeed");

        let tail = job
            .planned_components
            .iter()
            .find(|c| c.name == "tail")
            .expect("expected tail");
        let preserved = tail
            .attach_to
            .as_ref()
            .and_then(|att| att.joint.as_ref())
            .expect("expected joint to be preserved");
        assert_eq!(preserved.kind, AiJointKindJson::Hinge);
        assert_eq!(preserved.axis_join, hinge.axis_join);
        assert_eq!(preserved.limits_degrees, hinge.limits_degrees);
    }
}
