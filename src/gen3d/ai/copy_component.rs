use bevy::prelude::*;
use std::collections::HashMap;

use crate::object::registry::{
    builtin_object_id, AnchorDef, ColliderProfile, ObjectDef, ObjectInteraction, ObjectPartDef,
    ObjectPartKind,
};

use super::super::state::Gen3dDraft;
use super::super::{GEN3D_MAX_COMPONENTS, GEN3D_MAX_PARTS};
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

fn invert_transform(t: Transform) -> Option<Transform> {
    let inv = t.to_matrix().inverse();
    let (scale, rotation, translation) = inv.to_scale_rotation_translation();
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

fn anchor_transform_from_defs(anchors: &[AnchorDef], name: &str) -> Transform {
    if name == "origin" {
        return Transform::IDENTITY;
    }
    anchors
        .iter()
        .find(|a| a.name.as_ref() == name)
        .map(|a| a.transform)
        .unwrap_or(Transform::IDENTITY)
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

    let preserve_target_alignment_delta =
        if anchors_mode == Gen3dCopyAnchorsMode::PreserveTargetAnchors {
            let source_child_anchor = components[source_idx]
                .attach_to
                .as_ref()
                .map(|att| att.child_anchor.as_str());
            let target_child_anchor = components[target_idx]
                .attach_to
                .as_ref()
                .map(|att| att.child_anchor.as_str());
            if let (Some(source_child_anchor), Some(target_child_anchor)) =
                (source_child_anchor, target_child_anchor)
            {
                let source_anchor =
                    anchor_transform_from_defs(&source_def.anchors, source_child_anchor);
                let target_anchor =
                    anchor_transform_from_defs(&target_def.anchors, target_child_anchor);
                let inv_source_anchor = invert_transform(source_anchor);
                inv_source_anchor.and_then(|inv| compose_transform(target_anchor, inv))
            } else {
                None
            }
        } else {
            None
        };

    let (new_geometry_parts, new_size, new_anchors, mode_used) = match mode {
        Gen3dCopyMode::Detached => {
            let delta_parts = preserve_target_alignment_delta
                .and_then(|align| compose_transform(delta, align))
                .unwrap_or(delta);
            let mut parts = strip_attach_refs(&source_def.parts);
            apply_delta_to_parts(&mut parts, delta_parts);
            let anchors = match anchors_mode {
                Gen3dCopyAnchorsMode::CopySourceAnchors => {
                    apply_delta_to_anchors(&source_def.anchors, delta)
                }
                Gen3dCopyAnchorsMode::PreserveTargetAnchors => target_def.anchors.clone(),
            };
            let size = transformed_size(source_def.size, delta_parts);
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
    components: &mut Vec<Gen3dPlannedComponent>,
    draft: &mut Gen3dDraft,
    source_root_idx: usize,
    target_root_idx: usize,
    mode: Gen3dCopyMode,
    anchors_mode: Gen3dCopyAnchorsMode,
    delta: Transform,
) -> Result<Vec<Gen3dCopyComponentOutcome>, String> {
    ensure_target_subtree_shape(components, draft, source_root_idx, target_root_idx)?;

    let children = build_children_map(components);
    let pairs = map_subtree_pairs(components, &children, source_root_idx, target_root_idx)?;

    let mut out: Vec<Gen3dCopyComponentOutcome> = Vec::new();
    for (source_idx, target_idx) in pairs {
        let outcome = copy_component_into(
            components.as_mut_slice(),
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

fn ensure_target_subtree_shape(
    components: &mut Vec<Gen3dPlannedComponent>,
    draft: &mut Gen3dDraft,
    source_root_idx: usize,
    target_root_idx: usize,
) -> Result<(), String> {
    if source_root_idx >= components.len() || target_root_idx >= components.len() {
        return Err("copy_component_subtree: root index out of range".into());
    }
    if source_root_idx == target_root_idx {
        return Err("copy_component_subtree: source_root and target_root are identical".into());
    }

    let target_root_name = components[target_root_idx].name.clone();

    fn def_for_component<'a>(draft: &'a mut Gen3dDraft, name: &str) -> Option<&'a mut ObjectDef> {
        let id = component_object_id(name);
        draft.defs.iter_mut().find(|d| d.object_id == id)
    }

    fn ensure_stub_def_exists(draft: &mut Gen3dDraft, component: &Gen3dPlannedComponent) {
        let id = component_object_id(&component.name);
        if draft.defs.iter().any(|d| d.object_id == id) {
            return;
        }
        let size = component.planned_size.abs().max(Vec3::splat(0.01));
        draft.defs.push(ObjectDef {
            object_id: id,
            label: format!("gen3d_component_{}", component.name).into(),
            size,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: component.anchors.clone(),
            parts: Vec::new(),
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        });
    }

    fn allocate_unique_name(components: &[Gen3dPlannedComponent], proposed: String) -> String {
        if !components.iter().any(|c| c.name == proposed) {
            return proposed;
        }
        for i in 2..=999u32 {
            let candidate = format!("{proposed}_{i}");
            if !components.iter().any(|c| c.name == candidate) {
                return candidate;
            }
        }
        format!("{proposed}_999")
    }

    fn clone_missing_subtree(
        components: &mut Vec<Gen3dPlannedComponent>,
        draft: &mut Gen3dDraft,
        source_idx: usize,
        target_parent_idx: usize,
        target_root_name: &str,
        source_children_map: &[Vec<usize>],
    ) -> Result<usize, String> {
        if components.len() >= GEN3D_MAX_COMPONENTS {
            return Err(format!(
                "copy_component_subtree: cannot expand subtree (max components {GEN3D_MAX_COMPONENTS} reached)"
            ));
        }

        let source = components[source_idx].clone();
        let parent_name = components[target_parent_idx].name.clone();
        let mut new_comp = source.clone();
        let proposed_name = format!("{target_root_name}__{}", source.name);
        new_comp.name = allocate_unique_name(components, proposed_name);
        new_comp.display_name = new_comp.name.clone();
        new_comp.pos = Vec3::ZERO;
        new_comp.rot = Quat::IDENTITY;
        new_comp.actual_size = None;
        if let Some(mut att) = new_comp.attach_to.clone() {
            att.parent = parent_name;
            new_comp.attach_to = Some(att);
        } else {
            return Err(format!(
                "copy_component_subtree: source component `{}` is missing attach_to; cannot clone under `{}`",
                source.name, components[target_parent_idx].name
            ));
        }

        let new_idx = components.len();
        components.push(new_comp.clone());
        ensure_stub_def_exists(draft, &new_comp);

        // Add attachment reference from parent -> new child in the draft.
        let Some(att) = new_comp.attach_to.as_ref() else {
            return Err(
                "copy_component_subtree: internal error: missing attach_to after clone".into(),
            );
        };
        let parent_anchor = att.parent_anchor.as_str();
        if parent_anchor != "origin"
            && !components[target_parent_idx]
                .anchors
                .iter()
                .any(|a| a.name.as_ref() == parent_anchor)
        {
            return Err(format!(
                "copy_component_subtree: cannot attach cloned component `{}` to `{}`: missing parent_anchor `{}`",
                new_comp.name, components[target_parent_idx].name, parent_anchor
            ));
        }

        let child_id = component_object_id(&new_comp.name);
        let attachment = crate::object::registry::AttachmentDef {
            parent_anchor: att.parent_anchor.clone().into(),
            child_anchor: att.child_anchor.clone().into(),
        };
        let mut part = ObjectPartDef::object_ref(child_id, att.offset).with_attachment(attachment);
        part.animations.extend(att.animations.clone());
        if let Some(parent_def) = def_for_component(draft, &components[target_parent_idx].name) {
            parent_def.parts.push(part);
        }

        // Recursively clone descendants.
        for &source_child in source_children_map[source_idx].iter() {
            let _ = clone_missing_subtree(
                components,
                draft,
                source_child,
                new_idx,
                target_root_name,
                source_children_map,
            )?;
        }

        Ok(new_idx)
    }

    fn rec(
        components: &mut Vec<Gen3dPlannedComponent>,
        draft: &mut Gen3dDraft,
        source_idx: usize,
        target_idx: usize,
        target_root_name: &str,
    ) -> Result<(), String> {
        let children = build_children_map(components);
        let source_children = children.get(source_idx).cloned().unwrap_or_default();
        let target_children = children.get(target_idx).cloned().unwrap_or_default();

        if target_children.len() > source_children.len() {
            return Err(format!(
                "copy_component_subtree: subtree shape mismatch at `{}` ({} children) vs `{}` ({} children)",
                components[source_idx].name,
                source_children.len(),
                components[target_idx].name,
                target_children.len()
            ));
        }

        if target_children.len() < source_children.len() {
            if !target_children.is_empty() {
                return Err(format!(
                    "copy_component_subtree: subtree shape mismatch at `{}` ({} children) vs `{}` ({} children). Target subtree is partially populated; expand the plan to match or delete the existing target descendants.",
                    components[source_idx].name,
                    source_children.len(),
                    components[target_idx].name,
                    target_children.len()
                ));
            }

            // Clone the full missing branch under this target node.
            let source_children_map = children;
            for &s_child in source_children.iter() {
                let _ = clone_missing_subtree(
                    components,
                    draft,
                    s_child,
                    target_idx,
                    target_root_name,
                    &source_children_map,
                )?;
            }
            return Ok(());
        }

        for (s_child, t_child) in source_children
            .iter()
            .copied()
            .zip(target_children.iter().copied())
        {
            rec(components, draft, s_child, t_child, target_root_name)?;
        }
        Ok(())
    }

    rec(
        components,
        draft,
        source_root_idx,
        target_root_idx,
        &target_root_name,
    )
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

    #[test]
    fn preserve_target_copy_aligns_geometry_to_mount_anchor() {
        let mut components = vec![stub_component("a"), stub_component("b")];
        components[0].actual_size = Some(Vec3::ONE);
        components[0].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "root".into(),
            parent_anchor: "origin".into(),
            child_anchor: "mount".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });
        components[1].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "root".into(),
            parent_anchor: "origin".into(),
            child_anchor: "mount".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });

        let a_id = component_object_id("a");
        let b_id = component_object_id("b");

        let source_def = ObjectDef {
            object_id: a_id,
            label: "a".into(),
            size: Vec3::ONE,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: vec![anchor_named_forward("mount", Vec3::Z)],
            parts: vec![ObjectPartDef::primitive(
                crate::object::registry::PrimitiveVisualDef::Primitive {
                    mesh: crate::object::registry::MeshKey::UnitCube,
                    params: None,
                    color: Color::WHITE,
                    unlit: false,
                },
                Transform::from_translation(Vec3::new(0.0, 0.0, 1.0)),
            )],
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        };

        let target_def = ObjectDef {
            object_id: b_id,
            label: "b".into(),
            size: Vec3::ONE,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: vec![anchor_named_forward("mount", Vec3::X)],
            parts: Vec::new(),
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        };

        let mut draft = Gen3dDraft::default();
        draft.defs = vec![source_def.clone(), target_def.clone()];

        copy_component_into(
            &mut components,
            &mut draft,
            0,
            1,
            Gen3dCopyMode::Detached,
            Gen3dCopyAnchorsMode::PreserveTargetAnchors,
            Transform::IDENTITY,
        )
        .expect("copy ok");

        let b_def_after = draft.defs.iter().find(|d| d.object_id == b_id).unwrap();
        let part = b_def_after
            .parts
            .iter()
            .find(|p| matches!(p.kind, ObjectPartKind::Primitive { .. }))
            .expect("copied primitive");

        let source_anchor = source_def.anchors[0].transform.to_matrix();
        let target_anchor = b_def_after.anchors[0].transform.to_matrix();
        let rel_source = source_anchor.inverse() * source_def.parts[0].transform.to_matrix();
        let rel_target = target_anchor.inverse() * part.transform.to_matrix();

        let dp = (rel_source.w_axis.truncate() - rel_target.w_axis.truncate()).length();
        assert!(
            dp < 1e-3,
            "expected copied part transform relative to mount anchor to be preserved; delta={dp}"
        );
    }

    #[test]
    fn subtree_copy_can_expand_missing_target_descendants() {
        // Source subtree: leg_0_root -> leg_0_upper -> leg_0_lower -> leg_0_foot (generated).
        // Target subtree: leg_1_root exists but has NO descendants in the plan. Subtree copy should
        // expand the plan/draft with missing descendants under leg_1_root, then copy geometry.
        let mut components = vec![
            stub_component("body"),
            stub_component("leg_0_root"),
            stub_component("leg_0_upper"),
            stub_component("leg_0_lower"),
            stub_component("leg_0_foot"),
            stub_component("leg_1_root"),
        ];
        components[0].anchors = vec![
            anchor_named_forward("leg_0_mount", Vec3::Z),
            anchor_named_forward("leg_1_mount", Vec3::Z),
        ];
        components[1].anchors = vec![
            anchor_named_forward("to_body", Vec3::Z),
            anchor_named_forward("to_upper", Vec3::Z),
        ];
        components[2].anchors = vec![
            anchor_named_forward("to_root", Vec3::Z),
            anchor_named_forward("to_lower", Vec3::Z),
        ];
        components[3].anchors = vec![
            anchor_named_forward("to_upper", Vec3::Z),
            anchor_named_forward("to_foot", Vec3::Z),
        ];
        components[4].anchors = vec![anchor_named_forward("to_lower", Vec3::Z)];
        components[5].anchors = vec![
            anchor_named_forward("to_body", Vec3::X),
            anchor_named_forward("to_upper", Vec3::Z),
        ];

        components[1].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "body".into(),
            parent_anchor: "leg_0_mount".into(),
            child_anchor: "to_body".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });
        components[2].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "leg_0_root".into(),
            parent_anchor: "to_upper".into(),
            child_anchor: "to_root".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });
        components[3].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "leg_0_upper".into(),
            parent_anchor: "to_lower".into(),
            child_anchor: "to_upper".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });
        components[4].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "leg_0_lower".into(),
            parent_anchor: "to_foot".into(),
            child_anchor: "to_lower".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });
        components[5].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "body".into(),
            parent_anchor: "leg_1_mount".into(),
            child_anchor: "to_body".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });

        // Mark source chain as generated.
        components[1].actual_size = Some(Vec3::ONE);
        components[2].actual_size = Some(Vec3::ONE);
        components[3].actual_size = Some(Vec3::ONE);
        components[4].actual_size = Some(Vec3::ONE);

        let body_id = component_object_id("body");
        let leg0_root_id = component_object_id("leg_0_root");
        let leg0_upper_id = component_object_id("leg_0_upper");
        let leg0_lower_id = component_object_id("leg_0_lower");
        let leg0_foot_id = component_object_id("leg_0_foot");
        let leg1_root_id = component_object_id("leg_1_root");

        let mut draft = Gen3dDraft::default();
        draft.defs = vec![
            {
                let mut def = stub_def(body_id, "body");
                def.anchors = vec![
                    anchor_named_forward("leg_0_mount", Vec3::Z),
                    anchor_named_forward("leg_1_mount", Vec3::Z),
                ];
                def
            },
            {
                let mut def = stub_def(leg0_root_id, "leg_0_root");
                def.anchors = vec![
                    anchor_named_forward("to_body", Vec3::Z),
                    anchor_named_forward("to_upper", Vec3::Z),
                ];
                def
            },
            {
                let mut def = stub_def(leg0_upper_id, "leg_0_upper");
                def.anchors = vec![
                    anchor_named_forward("to_root", Vec3::Z),
                    anchor_named_forward("to_lower", Vec3::Z),
                ];
                def
            },
            {
                let mut def = stub_def(leg0_lower_id, "leg_0_lower");
                def.anchors = vec![
                    anchor_named_forward("to_upper", Vec3::Z),
                    anchor_named_forward("to_foot", Vec3::Z),
                ];
                def
            },
            {
                let mut def = stub_def(leg0_foot_id, "leg_0_foot");
                def.anchors = vec![anchor_named_forward("to_lower", Vec3::Z)];
                def
            },
            {
                let mut def = stub_def(leg1_root_id, "leg_1_root");
                def.anchors = vec![
                    anchor_named_forward("to_body", Vec3::X),
                    anchor_named_forward("to_upper", Vec3::Z),
                ];
                // No attachment refs under leg_1_root initially.
                def.parts.clear();
                def
            },
        ];

        copy_component_subtree_into(
            &mut components,
            &mut draft,
            1,
            5,
            Gen3dCopyMode::Detached,
            Gen3dCopyAnchorsMode::PreserveTargetAnchors,
            Transform::IDENTITY,
        )
        .expect("subtree copy ok");

        // After expansion, leg_1_root must have at least one attachment child part.
        let leg1_def_after = draft
            .defs
            .iter()
            .find(|d| d.object_id == leg1_root_id)
            .unwrap();
        assert!(
            leg1_def_after
                .parts
                .iter()
                .any(|p| matches!(p.kind, ObjectPartKind::ObjectRef { .. })
                    && p.attachment.is_some()),
            "expected expanded subtree to attach descendants under leg_1_root"
        );

        // And we should have at least one new component def created with primitives copied in.
        let generated_new = draft.defs.iter().any(|d| {
            d.label.as_ref().starts_with("gen3d_component_leg_1_root__")
                && d.parts
                    .iter()
                    .any(|p| matches!(p.kind, ObjectPartKind::Primitive { .. }))
        });
        assert!(
            generated_new,
            "expected subtree copy to create + populate new descendant component defs under leg_1_root"
        );
    }
}
