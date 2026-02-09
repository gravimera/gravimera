use bevy::prelude::*;
use std::collections::HashMap;

use crate::object::registry::{
    builtin_object_id, AnchorDef, ObjectDef, ObjectPartDef, ObjectPartKind,
};

use super::super::state::Gen3dDraft;
use super::super::GEN3D_MAX_PARTS;
use super::Gen3dPlannedComponent;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Gen3dCopyMode {
    Detached,
    Linked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Gen3dCopyAnchorsMode {
    CopySourceAnchors,
    PreserveTargetAnchors,
}

#[derive(Clone, Debug)]
pub(super) struct Gen3dCopyComponentOutcome {
    pub(super) source_component_name: String,
    pub(super) target_component_name: String,
    pub(super) mode_used: Gen3dCopyMode,
}

fn is_attachment_object_ref(part: &ObjectPartDef) -> bool {
    matches!(part.kind, ObjectPartKind::ObjectRef { .. }) && part.attachment.is_some()
}

fn component_object_id(name: &str) -> u128 {
    builtin_object_id(&format!("gravimera/gen3d/component/{}", name))
}

fn compose_transform(parent: Transform, child: Transform) -> Option<Transform> {
    let composed = parent.to_matrix() * child.to_matrix();
    let (scale, rotation, translation) = composed.to_scale_rotation_translation();
    if !scale.is_finite() || !rotation.is_finite() || !translation.is_finite() {
        return None;
    }
    Some(Transform {
        translation,
        rotation,
        scale,
    })
}

fn rotated_half_extents(half: Vec3, rotation: Quat) -> Vec3 {
    let abs = Mat3::from_quat(rotation).abs();
    abs * half
}

fn transformed_size(base_size: Vec3, delta: Transform) -> Vec3 {
    let base = base_size.abs().max(Vec3::splat(0.01));
    let scale = delta.scale.abs().max(Vec3::splat(0.01));
    let half = base * 0.5 * scale;
    let rot = if delta.rotation.is_finite() {
        delta.rotation.normalize()
    } else {
        Quat::IDENTITY
    };
    (rotated_half_extents(half, rot) * 2.0)
        .abs()
        .max(Vec3::splat(0.01))
}

fn is_leaf_component(components: &[Gen3dPlannedComponent], name: &str) -> bool {
    !components.iter().any(|c| {
        c.attach_to
            .as_ref()
            .is_some_and(|att| att.parent.as_str() == name)
    })
}

fn strip_attach_refs(parts: &[ObjectPartDef]) -> Vec<ObjectPartDef> {
    parts
        .iter()
        .filter(|p| !is_attachment_object_ref(p))
        .cloned()
        .collect()
}

fn keep_attach_refs(parts: &[ObjectPartDef]) -> Vec<ObjectPartDef> {
    parts
        .iter()
        .filter(|p| is_attachment_object_ref(p))
        .cloned()
        .collect()
}

fn apply_delta_to_parts(parts: &mut [ObjectPartDef], delta: Transform) {
    for part in parts.iter_mut() {
        if let Some(t) = compose_transform(delta, part.transform) {
            part.transform = t;
        }
    }
}

fn apply_delta_to_anchors(anchors: &[AnchorDef], delta: Transform) -> Vec<AnchorDef> {
    anchors
        .iter()
        .map(|a| AnchorDef {
            name: a.name.clone(),
            transform: compose_transform(delta, a.transform).unwrap_or(a.transform),
        })
        .collect()
}

fn linked_copy_geometry_part(source_object_id: u128, delta: Transform) -> ObjectPartDef {
    ObjectPartDef::object_ref(source_object_id, delta)
}

fn is_linked_copy_def(def: &ObjectDef) -> Option<(u128, Transform)> {
    let geometry = strip_attach_refs(&def.parts);
    if geometry.len() != 1 {
        return None;
    }
    let part = &geometry[0];
    if part.attachment.is_some() {
        return None;
    }
    match part.kind {
        ObjectPartKind::ObjectRef { object_id } => Some((object_id, part.transform)),
        _ => None,
    }
}

pub(super) fn copy_component_into(
    components: &mut [Gen3dPlannedComponent],
    draft: &mut Gen3dDraft,
    source_idx: usize,
    target_idx: usize,
    mode: Gen3dCopyMode,
    anchors_mode: Gen3dCopyAnchorsMode,
    delta: Transform,
) -> Result<Gen3dCopyComponentOutcome, String> {
    if source_idx >= components.len() || target_idx >= components.len() {
        return Err("copy_component_into: component index out of range".into());
    }
    if source_idx == target_idx {
        return Err("copy_component_into: source and target are the same component".into());
    }
    let source_name = components[source_idx].name.clone();
    let target_name = components[target_idx].name.clone();

    if components[source_idx].actual_size.is_none() {
        return Err(format!(
            "copy_component_into: source component `{source_name}` is not generated yet"
        ));
    }

    let source_id = component_object_id(&source_name);
    let target_id = component_object_id(&target_name);

    let Some(source_def) = draft
        .defs
        .iter()
        .find(|d| d.object_id == source_id)
        .cloned()
    else {
        return Err(format!(
            "copy_component_into: missing source object def for `{source_name}`"
        ));
    };
    let Some(target_def) = draft
        .defs
        .iter()
        .find(|d| d.object_id == target_id)
        .cloned()
    else {
        return Err(format!(
            "copy_component_into: missing target object def for `{target_name}`"
        ));
    };

    let preserved_attachments = keep_attach_refs(&target_def.parts);

    let (new_geometry_parts, new_size, new_anchors, mode_used) = match mode {
        Gen3dCopyMode::Detached => {
            let mut parts = strip_attach_refs(&source_def.parts);
            apply_delta_to_parts(&mut parts, delta);
            let anchors = match anchors_mode {
                Gen3dCopyAnchorsMode::CopySourceAnchors => {
                    apply_delta_to_anchors(&source_def.anchors, delta)
                }
                Gen3dCopyAnchorsMode::PreserveTargetAnchors => target_def.anchors.clone(),
            };
            let size = transformed_size(source_def.size, delta);
            (parts, size, anchors, Gen3dCopyMode::Detached)
        }
        Gen3dCopyMode::Linked => {
            if !is_leaf_component(components, &source_name) {
                return Err(format!(
                    "copy_component_into: linked copies require the SOURCE component to be a leaf (no child attachments). `{source_name}` has children; use mode=detached."
                ));
            }
            let parts = vec![linked_copy_geometry_part(source_id, delta)];
            // A linked copy shares geometry but keeps the target's interface anchors (often mirrored).
            // Overwriting anchors from the source breaks the attachment join frame rule and can flip
            // spins (e.g., vehicle wheels).
            let anchors = target_def.anchors.clone();
            let size = transformed_size(source_def.size, delta);
            (parts, size, anchors, Gen3dCopyMode::Linked)
        }
    };

    let part_count = new_geometry_parts.len();
    if part_count == 0 {
        return Err("copy_component_into: produced 0 geometry parts".into());
    }
    if part_count > GEN3D_MAX_PARTS {
        return Err(format!(
            "copy_component_into: produced too many parts: {part_count} (max {GEN3D_MAX_PARTS})"
        ));
    }

    let mut merged_parts = new_geometry_parts;
    merged_parts.extend(preserved_attachments);

    let mut new_def = target_def;
    new_def.size = new_size;
    new_def.anchors = new_anchors.clone();
    new_def.parts = merged_parts;

    if let Some(existing) = draft.defs.iter_mut().find(|d| d.object_id == target_id) {
        *existing = new_def;
    } else {
        draft.defs.push(new_def);
    }

    components[target_idx].actual_size = Some(new_size);
    components[target_idx].anchors = new_anchors;

    Ok(Gen3dCopyComponentOutcome {
        source_component_name: source_name,
        target_component_name: target_name,
        mode_used,
    })
}

pub(super) fn detach_component_copy(
    components: &mut [Gen3dPlannedComponent],
    draft: &mut Gen3dDraft,
    target_idx: usize,
) -> Result<Gen3dCopyComponentOutcome, String> {
    if target_idx >= components.len() {
        return Err("detach_component_copy: component index out of range".into());
    }
    let target_name = components[target_idx].name.clone();
    let target_id = component_object_id(&target_name);

    let Some(target_def) = draft
        .defs
        .iter()
        .find(|d| d.object_id == target_id)
        .cloned()
    else {
        return Err(format!(
            "detach_component_copy: missing target object def for `{target_name}`"
        ));
    };

    let Some((source_object_id, internal_delta)) = is_linked_copy_def(&target_def) else {
        return Err(format!(
            "detach_component_copy: component `{target_name}` is not a linked copy"
        ));
    };

    let Some(source_def) = draft
        .defs
        .iter()
        .find(|d| d.object_id == source_object_id)
        .cloned()
    else {
        return Err(format!(
            "detach_component_copy: missing source object def {source_object_id:#x}"
        ));
    };

    let preserved_attachments = keep_attach_refs(&target_def.parts);
    let preserved_anchors = target_def.anchors.clone();

    let mut parts = strip_attach_refs(&source_def.parts);
    apply_delta_to_parts(&mut parts, internal_delta);
    // Detaching should keep the target's anchors stable (often mirrored), while materializing
    // the geometry so it can be edited independently.
    let anchors = preserved_anchors;
    let size = transformed_size(source_def.size, internal_delta);

    let part_count = parts.len();
    if part_count == 0 {
        return Err("detach_component_copy: produced 0 geometry parts".into());
    }
    if part_count > GEN3D_MAX_PARTS {
        return Err(format!(
            "detach_component_copy: produced too many parts: {part_count} (max {GEN3D_MAX_PARTS})"
        ));
    }

    let mut merged_parts = parts;
    merged_parts.extend(preserved_attachments);

    let mut new_def = target_def;
    new_def.size = size;
    new_def.anchors = anchors.clone();
    new_def.parts = merged_parts;

    if let Some(existing) = draft.defs.iter_mut().find(|d| d.object_id == target_id) {
        *existing = new_def;
    } else {
        draft.defs.push(new_def);
    }

    components[target_idx].actual_size = Some(size);
    components[target_idx].anchors = anchors;

    let source_name = components
        .iter()
        .find(|c| component_object_id(&c.name) == source_object_id)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| format!("{source_object_id:#x}"));

    Ok(Gen3dCopyComponentOutcome {
        source_component_name: source_name,
        target_component_name: target_name,
        mode_used: Gen3dCopyMode::Detached,
    })
}

fn build_children_map(components: &[Gen3dPlannedComponent]) -> Vec<Vec<usize>> {
    let mut name_to_idx: HashMap<String, usize> = HashMap::new();
    for (idx, comp) in components.iter().enumerate() {
        name_to_idx.insert(comp.name.clone(), idx);
    }

    let mut children: Vec<Vec<usize>> = vec![Vec::new(); components.len()];
    for (idx, comp) in components.iter().enumerate() {
        let Some(att) = comp.attach_to.as_ref() else {
            continue;
        };
        let Some(&parent_idx) = name_to_idx.get(att.parent.as_str()) else {
            continue;
        };
        children[parent_idx].push(idx);
    }

    for parent_idx in 0..children.len() {
        children[parent_idx].sort_by(|&a, &b| {
            let a_att = components[a]
                .attach_to
                .as_ref()
                .expect("child must have attach_to");
            let b_att = components[b]
                .attach_to
                .as_ref()
                .expect("child must have attach_to");
            (
                a_att.parent_anchor.as_str(),
                a_att.child_anchor.as_str(),
                components[a].name.as_str(),
            )
                .cmp(&(
                    b_att.parent_anchor.as_str(),
                    b_att.child_anchor.as_str(),
                    components[b].name.as_str(),
                ))
        });
    }

    children
}

fn map_subtree_pairs(
    components: &[Gen3dPlannedComponent],
    children: &[Vec<usize>],
    source_root_idx: usize,
    target_root_idx: usize,
) -> Result<Vec<(usize, usize)>, String> {
    if source_root_idx >= components.len() || target_root_idx >= components.len() {
        return Err("map_subtree_pairs: root index out of range".into());
    }
    if source_root_idx == target_root_idx {
        return Err("map_subtree_pairs: source and target roots are identical".into());
    }

    let mut pairs: Vec<(usize, usize)> = Vec::new();
    let mut visiting_source = vec![false; components.len()];
    let mut visiting_target = vec![false; components.len()];

    fn rec(
        components: &[Gen3dPlannedComponent],
        children: &[Vec<usize>],
        source_idx: usize,
        target_idx: usize,
        pairs: &mut Vec<(usize, usize)>,
        visiting_source: &mut [bool],
        visiting_target: &mut [bool],
    ) -> Result<(), String> {
        if visiting_source[source_idx] {
            return Err(format!(
                "copy_component_subtree: cycle detected in source subtree at `{}`",
                components[source_idx].name
            ));
        }
        if visiting_target[target_idx] {
            return Err(format!(
                "copy_component_subtree: cycle detected in target subtree at `{}`",
                components[target_idx].name
            ));
        }

        visiting_source[source_idx] = true;
        visiting_target[target_idx] = true;

        pairs.push((source_idx, target_idx));

        let source_children = &children[source_idx];
        let target_children = &children[target_idx];
        if source_children.len() != target_children.len() {
            return Err(format!(
                "copy_component_subtree: subtree shape mismatch at `{}` ({} children) vs `{}` ({} children)",
                components[source_idx].name,
                source_children.len(),
                components[target_idx].name,
                target_children.len()
            ));
        }

        for (s_child, t_child) in source_children
            .iter()
            .copied()
            .zip(target_children.iter().copied())
        {
            rec(
                components,
                children,
                s_child,
                t_child,
                pairs,
                visiting_source,
                visiting_target,
            )?;
        }

        visiting_source[source_idx] = false;
        visiting_target[target_idx] = false;
        Ok(())
    }

    rec(
        components,
        children,
        source_root_idx,
        target_root_idx,
        &mut pairs,
        &mut visiting_source,
        &mut visiting_target,
    )?;

    Ok(pairs)
}

pub(super) fn copy_component_subtree_into(
    components: &mut [Gen3dPlannedComponent],
    draft: &mut Gen3dDraft,
    source_root_idx: usize,
    target_root_idx: usize,
    mode: Gen3dCopyMode,
    anchors_mode: Gen3dCopyAnchorsMode,
    delta: Transform,
) -> Result<Vec<Gen3dCopyComponentOutcome>, String> {
    let children = build_children_map(components);
    let pairs = map_subtree_pairs(components, &children, source_root_idx, target_root_idx)?;

    let mut out: Vec<Gen3dCopyComponentOutcome> = Vec::new();
    for (source_idx, target_idx) in pairs {
        let outcome = copy_component_into(
            components,
            draft,
            source_idx,
            target_idx,
            mode,
            anchors_mode,
            delta,
        )?;
        out.push(outcome);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::registry::{ColliderProfile, ObjectInteraction};

    fn anchor_named_forward(name: &str, forward: Vec3) -> AnchorDef {
        let rot = super::super::convert::plan_rotation_from_forward_up(forward, Some(Vec3::Y));
        AnchorDef {
            name: name.to_string().into(),
            transform: Transform {
                rotation: rot,
                ..default()
            },
        }
    }

    fn stub_component(name: &str) -> Gen3dPlannedComponent {
        Gen3dPlannedComponent {
            display_name: name.to_string(),
            name: name.to_string(),
            purpose: String::new(),
            modeling_notes: String::new(),
            pos: Vec3::ZERO,
            rot: Quat::IDENTITY,
            planned_size: Vec3::ONE,
            actual_size: None,
            anchors: vec![AnchorDef {
                name: "origin".into(),
                transform: Transform::IDENTITY,
            }],
            contacts: Vec::new(),
            attach_to: None,
        }
    }

    fn stub_def(object_id: u128, name: &str) -> ObjectDef {
        ObjectDef {
            object_id,
            label: name.to_string().into(),
            size: Vec3::ONE,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: vec![AnchorDef {
                name: "origin".into(),
                transform: Transform::IDENTITY,
            }],
            parts: vec![ObjectPartDef::primitive(
                crate::object::registry::PrimitiveVisualDef::Primitive {
                    mesh: crate::object::registry::MeshKey::UnitCube,
                    params: None,
                    color: Color::WHITE,
                    unlit: false,
                },
                Transform::from_scale(Vec3::splat(1.0)),
            )],
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        }
    }

    #[test]
    fn detached_copy_preserves_target_attachment_refs() {
        let mut components = vec![stub_component("a"), stub_component("b")];
        components[0].actual_size = Some(Vec3::ONE);

        let a_id = component_object_id("a");
        let b_id = component_object_id("b");
        let child_id = builtin_object_id("gravimera/gen3d/component/child");

        let mut draft = Gen3dDraft::default();
        draft.defs = vec![
            stub_def(a_id, "a"),
            ObjectDef {
                parts: vec![ObjectPartDef::object_ref(child_id, Transform::IDENTITY)
                    .with_attachment(crate::object::registry::AttachmentDef {
                        parent_anchor: "origin".into(),
                        child_anchor: "origin".into(),
                    })],
                ..stub_def(b_id, "b")
            },
            stub_def(child_id, "child"),
        ];

        copy_component_into(
            &mut components,
            &mut draft,
            0,
            1,
            Gen3dCopyMode::Detached,
            Gen3dCopyAnchorsMode::CopySourceAnchors,
            Transform::IDENTITY,
        )
        .expect("copy ok");

        let b_def = draft.defs.iter().find(|d| d.object_id == b_id).unwrap();
        assert!(
            b_def
                .parts
                .iter()
                .any(|p| matches!(p.kind, ObjectPartKind::ObjectRef { .. })
                    && p.attachment.is_some()),
            "target attachment refs must be preserved"
        );
    }

    #[test]
    fn detach_materializes_linked_copy() {
        let mut components = vec![stub_component("a"), stub_component("b")];
        components[0].actual_size = Some(Vec3::ONE);
        components[1].actual_size = Some(Vec3::ONE);

        let a_id = component_object_id("a");
        let b_id = component_object_id("b");

        let mut draft = Gen3dDraft::default();
        draft.defs = vec![
            stub_def(a_id, "a"),
            ObjectDef {
                parts: vec![ObjectPartDef::object_ref(a_id, Transform::IDENTITY)],
                ..stub_def(b_id, "b")
            },
        ];

        detach_component_copy(&mut components, &mut draft, 1).expect("detach ok");
        let b_def = draft.defs.iter().find(|d| d.object_id == b_id).unwrap();
        assert!(
            b_def
                .parts
                .iter()
                .any(|p| matches!(p.kind, ObjectPartKind::Primitive { .. })),
            "detached copy must contain primitives"
        );
    }

    #[test]
    fn linked_copy_preserves_target_anchors() {
        let mut components = vec![stub_component("a"), stub_component("b")];
        components[0].actual_size = Some(Vec3::ONE);

        let a_id = component_object_id("a");
        let b_id = component_object_id("b");

        let mut a_def = stub_def(a_id, "a");
        a_def.anchors = vec![anchor_named_forward("axle", Vec3::NEG_X)];
        let mut b_def = stub_def(b_id, "b");
        b_def.anchors = vec![anchor_named_forward("axle", Vec3::X)];

        let mut draft = Gen3dDraft::default();
        draft.defs = vec![a_def, b_def];

        copy_component_into(
            &mut components,
            &mut draft,
            0,
            1,
            Gen3dCopyMode::Linked,
            Gen3dCopyAnchorsMode::CopySourceAnchors,
            Transform::IDENTITY,
        )
        .expect("linked copy ok");

        let b_def_after = draft.defs.iter().find(|d| d.object_id == b_id).unwrap();
        assert_eq!(b_def_after.anchors.len(), 1);
        let forward = b_def_after.anchors[0].transform.rotation * Vec3::Z;
        assert!(
            forward.dot(Vec3::X) > 0.99,
            "expected target anchor forward +X, got {forward:?}"
        );
    }

    #[test]
    fn detach_preserves_target_anchors() {
        let mut components = vec![stub_component("a"), stub_component("b")];
        components[0].actual_size = Some(Vec3::ONE);
        components[1].actual_size = Some(Vec3::ONE);

        let a_id = component_object_id("a");
        let b_id = component_object_id("b");

        let mut a_def = stub_def(a_id, "a");
        a_def.anchors = vec![anchor_named_forward("axle", Vec3::NEG_X)];
        let mut b_def = stub_def(b_id, "b");
        b_def.anchors = vec![anchor_named_forward("axle", Vec3::X)];
        b_def.parts = vec![ObjectPartDef::object_ref(a_id, Transform::IDENTITY)];

        let mut draft = Gen3dDraft::default();
        draft.defs = vec![a_def, b_def];

        detach_component_copy(&mut components, &mut draft, 1).expect("detach ok");

        let b_def_after = draft.defs.iter().find(|d| d.object_id == b_id).unwrap();
        assert_eq!(b_def_after.anchors.len(), 1);
        let forward = b_def_after.anchors[0].transform.rotation * Vec3::Z;
        assert!(
            forward.dot(Vec3::X) > 0.99,
            "expected target anchor forward +X, got {forward:?}"
        );
    }

    #[test]
    fn subtree_copy_preserves_target_anchors_and_attachment_refs() {
        let mut components = vec![
            stub_component("body"),
            stub_component("leg_a"),
            stub_component("foot_a"),
            stub_component("leg_b"),
            stub_component("foot_b"),
        ];

        components[1].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "body".into(),
            parent_anchor: "leg_mount_a".into(),
            child_anchor: "hip".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });
        components[2].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "leg_a".into(),
            parent_anchor: "ankle".into(),
            child_anchor: "hip".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });
        components[3].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "body".into(),
            parent_anchor: "leg_mount_b".into(),
            child_anchor: "hip".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });
        components[4].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "leg_b".into(),
            parent_anchor: "ankle".into(),
            child_anchor: "hip".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });

        // Source leg/foot are generated.
        components[1].actual_size = Some(Vec3::ONE);
        components[2].actual_size = Some(Vec3::ONE);

        let body_id = component_object_id("body");
        let leg_a_id = component_object_id("leg_a");
        let foot_a_id = component_object_id("foot_a");
        let leg_b_id = component_object_id("leg_b");
        let foot_b_id = component_object_id("foot_b");

        fn empty_def(mut def: ObjectDef) -> ObjectDef {
            def.parts.clear();
            def
        }

        let mut leg_a_def = stub_def(leg_a_id, "leg_a");
        leg_a_def.anchors = vec![anchor_named_forward("hip", Vec3::NEG_X)];
        let mut foot_a_def = stub_def(foot_a_id, "foot_a");
        foot_a_def.anchors = vec![anchor_named_forward("hip", Vec3::NEG_X)];

        let mut leg_b_def = empty_def(stub_def(leg_b_id, "leg_b"));
        leg_b_def.anchors = vec![anchor_named_forward("hip", Vec3::X)];
        leg_b_def.parts = vec![ObjectPartDef::object_ref(foot_b_id, Transform::IDENTITY)
            .with_attachment(crate::object::registry::AttachmentDef {
                parent_anchor: "hip".into(),
                child_anchor: "hip".into(),
            })];

        let mut foot_b_def = empty_def(stub_def(foot_b_id, "foot_b"));
        foot_b_def.anchors = vec![anchor_named_forward("hip", Vec3::X)];

        let mut draft = Gen3dDraft::default();
        draft.defs = vec![
            empty_def(stub_def(body_id, "body")),
            leg_a_def,
            foot_a_def,
            leg_b_def,
            foot_b_def,
        ];

        copy_component_subtree_into(
            &mut components,
            &mut draft,
            1,
            3,
            Gen3dCopyMode::Detached,
            Gen3dCopyAnchorsMode::PreserveTargetAnchors,
            Transform::IDENTITY,
        )
        .expect("subtree copy ok");

        let leg_b_after = draft.defs.iter().find(|d| d.object_id == leg_b_id).unwrap();
        assert!(
            leg_b_after
                .parts
                .iter()
                .any(|p| matches!(p.kind, ObjectPartKind::Primitive { .. })),
            "expected subtree copy to materialize primitives into leg_b"
        );

        let forward = leg_b_after.anchors[0].transform.rotation * Vec3::Z;
        assert!(
            forward.dot(Vec3::X) > 0.99,
            "expected subtree copy to preserve target hip forward +X, got {forward:?}"
        );

        assert!(
            leg_b_after
                .parts
                .iter()
                .any(|p| matches!(p.kind, ObjectPartKind::ObjectRef { .. })
                    && p.attachment.is_some()),
            "expected subtree copy to preserve target attachment refs"
        );

        let foot_b_after = draft
            .defs
            .iter()
            .find(|d| d.object_id == foot_b_id)
            .unwrap();
        assert!(
            foot_b_after
                .parts
                .iter()
                .any(|p| matches!(p.kind, ObjectPartKind::Primitive { .. })),
            "expected subtree copy to materialize primitives into foot_b"
        );
    }
}
