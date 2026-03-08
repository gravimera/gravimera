use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

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
    PreserveInterfaceAnchors,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Gen3dCopyAlignmentMode {
    Rotation,
    MirrorMountX,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Gen3dSubtreeCopyMissingBranchPolicy {
    /// Clone all missing target branches when a source child exists with the same attachment edge
    /// key. This matches the default expectations for explicit user tool calls.
    CloneAllMissing,
    /// Do not clone (and do not require) missing branches that contain components referenced by
    /// root-level interfaces (e.g. attack muzzle, aim components). This is intended for automatic
    /// symmetry reuse so we don't silently create non-functional duplicates.
    SkipExternallyReferenced,
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
    crate::geometry::mat4_to_transform_allow_degenerate_scale(composed)
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

fn transform_point(mat: Mat4, point: Vec3) -> Vec3 {
    (mat * point.extend(1.0)).truncate()
}

fn transform_dir(mat: Mat4, dir: Vec3) -> Vec3 {
    (mat * dir.extend(0.0)).truncate()
}

fn transform_from_mat4(mat: Mat4) -> Option<Transform> {
    crate::geometry::mat4_to_transform_allow_degenerate_scale(mat)
}

fn apply_delta_to_anchors(anchors: &[AnchorDef], delta: Transform) -> Vec<AnchorDef> {
    let delta_mat = delta.to_matrix();
    anchors
        .iter()
        .map(|a| {
            let pos = transform_point(delta_mat, a.transform.translation);
            let forward = transform_dir(delta_mat, a.transform.rotation * Vec3::Z);
            let up = transform_dir(delta_mat, a.transform.rotation * Vec3::Y);
            let rot = super::convert::plan_rotation_from_forward_up_lossy(forward, Some(up));
            AnchorDef {
                name: a.name.clone(),
                transform: Transform {
                    translation: pos,
                    rotation: rot,
                    scale: Vec3::ONE,
                },
            }
        })
        .collect()
}

fn anchor_transform_from_defs(anchors: &[AnchorDef], name: &str) -> Option<Transform> {
    if name == "origin" {
        return Some(Transform::IDENTITY);
    }
    anchors
        .iter()
        .find(|a| a.name.as_ref() == name)
        .map(|a| a.transform)
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
    alignment: Gen3dCopyAlignmentMode,
    delta: Transform,
    copied_target_indices: Option<&HashSet<usize>>,
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

    let preserve_target_alignment_delta = if matches!(
        anchors_mode,
        Gen3dCopyAnchorsMode::PreserveTargetAnchors
            | Gen3dCopyAnchorsMode::PreserveInterfaceAnchors
    ) {
        (|| -> Result<Option<Transform>, String> {
            let source_child_anchor = components[source_idx]
                .attach_to
                .as_ref()
                .map(|att| att.child_anchor.as_str());
            let target_child_anchor = components[target_idx]
                .attach_to
                .as_ref()
                .map(|att| att.child_anchor.as_str());
            let (Some(source_child_anchor), Some(target_child_anchor)) =
                (source_child_anchor, target_child_anchor)
            else {
                return Ok(None);
            };

            let source_anchor = anchor_transform_from_defs(&source_def.anchors, source_child_anchor)
                .ok_or_else(|| {
                    format!(
                        "copy_component_into: source component `{source_name}` is missing required anchor `{source_child_anchor}`"
                    )
                })?;
            let target_anchor = anchor_transform_from_defs(&target_def.anchors, target_child_anchor)
                .ok_or_else(|| {
                    format!(
                        "copy_component_into: target component `{target_name}` is missing required anchor `{target_child_anchor}`"
                    )
                })?;

            let inv_source_anchor = source_anchor.to_matrix().inverse();
            if !inv_source_anchor.is_finite() {
                return Ok(None);
            }

            // Align in the canonical JOIN frame, not directly from child_anchor→child_anchor.
            //
            // In Gen3D plans, child anchor bases are often adjusted to satisfy the engine's strict
            // "same-hemisphere" constraint (dot(parent.forward, child.forward) > 0, etc), while
            // an `attach_to.offset` rotation is used to compensate so the component's modeled
            // geometry still faces the intended direction.
            //
            // When we reuse geometry via copy, naively aligning by child anchors alone can apply a
            // second flip (baking it into the copied geometry/anchors) on top of the existing
            // attachment offset rotation, producing mirrored/doubled assemblies (e.g. a back arm's
            // rotor mount ends up on the wrong end).
            //
            // The offset is authored in the JOIN frame (parent anchor frame) and thus comparable
            // across attachments. Use it to compute an alignment that preserves the component's
            // effective join presentation:
            //   join -> child_local is (child_anchor * inv(offset)).
            // The source->target mapping through JOIN becomes:
            //   delta = child_anchor_target * inv(offset_target) * offset_source * inv(child_anchor_source)
            let source_offset = components[source_idx]
                .attach_to
                .as_ref()
                .map(|att| att.offset)
                .unwrap_or(Transform::IDENTITY);
            let target_offset = components[target_idx]
                .attach_to
                .as_ref()
                .map(|att| att.offset)
                .unwrap_or(Transform::IDENTITY);

            let source_offset_mat = source_offset.to_matrix();
            if !source_offset_mat.is_finite() {
                return Ok(None);
            }
            let inv_target_offset = target_offset.to_matrix().inverse();
            if !inv_target_offset.is_finite() {
                return Ok(None);
            }

            let target_mat = target_anchor.to_matrix();
            let rot_mat = target_mat * inv_target_offset * source_offset_mat * inv_source_anchor;
            // Optional mirrored alignment: flip the mount-local right axis (X) to preserve
            // the forward/up join convention while mirroring handedness (common for L/R reuse).
            let mirror_local = Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0));
            let mirror_mat =
                target_mat * mirror_local * inv_target_offset * source_offset_mat * inv_source_anchor;

            let chosen = match alignment {
                Gen3dCopyAlignmentMode::Rotation => rot_mat,
                Gen3dCopyAlignmentMode::MirrorMountX => mirror_mat,
            };
            Ok(transform_from_mat4(chosen))
        })()?
    } else {
        None
    };

    fn preserve_interface_anchor_names(
        components: &[Gen3dPlannedComponent],
        target_idx: usize,
        copied_target_indices: Option<&HashSet<usize>>,
    ) -> HashSet<String> {
        let mut out: HashSet<String> = HashSet::new();
        let Some(target) = components.get(target_idx) else {
            return out;
        };

        // Always preserve the mount interface (child_anchor) so join frames stay stable.
        if let Some(att) = target.attach_to.as_ref() {
            if !att.child_anchor.trim().is_empty() {
                out.insert(att.child_anchor.trim().to_string());
            }
        }

        // Preserve anchors used to attach children that are NOT being copied as part of this
        // operation. Otherwise, those children would "jump" in the assembly.
        let parent_name = target.name.as_str();
        for (child_idx, child) in components.iter().enumerate() {
            let Some(att) = child.attach_to.as_ref() else {
                continue;
            };
            if att.parent.as_str() != parent_name {
                continue;
            }
            let child_is_internal =
                copied_target_indices.is_some_and(|set| set.contains(&child_idx));
            if child_is_internal {
                continue;
            }
            if !att.parent_anchor.trim().is_empty() {
                out.insert(att.parent_anchor.trim().to_string());
            }
        }

        out
    }

    fn merge_anchors_preserve_interfaces(
        source_anchors: &[AnchorDef],
        target_anchors: &[AnchorDef],
        delta_parts: Transform,
        preserved: &HashSet<String>,
    ) -> Vec<AnchorDef> {
        let source_transformed = apply_delta_to_anchors(source_anchors, delta_parts);
        let mut source_by_name: HashMap<String, Transform> = HashMap::new();
        for anchor in source_transformed.iter() {
            source_by_name.insert(anchor.name.as_ref().to_string(), anchor.transform);
        }

        let mut seen: HashSet<String> = HashSet::new();
        let mut out: Vec<AnchorDef> = Vec::new();

        for target_anchor in target_anchors.iter() {
            let name = target_anchor.name.as_ref().to_string();
            seen.insert(name.clone());
            let transform = if preserved.contains(&name) {
                target_anchor.transform
            } else if let Some(source_transform) = source_by_name.get(&name) {
                *source_transform
            } else {
                target_anchor.transform
            };
            out.push(AnchorDef {
                name: target_anchor.name.clone(),
                transform,
            });
        }

        for source_anchor in source_transformed.into_iter() {
            let name = source_anchor.name.as_ref().to_string();
            if seen.contains(&name) {
                continue;
            }
            out.push(source_anchor);
        }

        out
    }

    let (new_geometry_parts, new_size, new_anchors, mode_used) = match mode {
        Gen3dCopyMode::Detached => {
            let delta_parts = preserve_target_alignment_delta
                .and_then(|align| compose_transform(delta, align))
                .unwrap_or(delta);
            let mut parts = strip_attach_refs(&source_def.parts);
            apply_delta_to_parts(&mut parts, delta_parts);
            let anchors = match anchors_mode {
                Gen3dCopyAnchorsMode::CopySourceAnchors => {
                    apply_delta_to_anchors(&source_def.anchors, delta_parts)
                }
                Gen3dCopyAnchorsMode::PreserveTargetAnchors => target_def.anchors.clone(),
                Gen3dCopyAnchorsMode::PreserveInterfaceAnchors => {
                    let preserved = preserve_interface_anchor_names(
                        components,
                        target_idx,
                        copied_target_indices,
                    );
                    merge_anchors_preserve_interfaces(
                        &source_def.anchors,
                        &target_def.anchors,
                        delta_parts,
                        &preserved,
                    )
                }
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
            let delta_parts = preserve_target_alignment_delta
                .and_then(|align| compose_transform(delta, align))
                .unwrap_or(delta);
            let parts = vec![linked_copy_geometry_part(source_id, delta_parts)];
            // A linked copy shares geometry but keeps the target's interface anchors (often mirrored).
            // Overwriting anchors from the source breaks the attachment join frame rule and can flip
            // spins (e.g., vehicle wheels).
            let anchors = target_def.anchors.clone();
            let size = transformed_size(source_def.size, delta_parts);
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

type AttachmentEdgeKey = (String, String);

fn attachment_edge_key_for_child(
    components: &[Gen3dPlannedComponent],
    child_idx: usize,
) -> Result<AttachmentEdgeKey, String> {
    let child = components
        .get(child_idx)
        .ok_or_else(|| "copy_component_subtree: child index out of range".to_string())?;
    let att = child
        .attach_to
        .as_ref()
        .ok_or_else(|| "copy_component_subtree: child component missing attach_to".to_string())?;
    Ok((att.parent_anchor.clone(), att.child_anchor.clone()))
}

fn children_by_attachment_edge_key(
    components: &[Gen3dPlannedComponent],
    parent_idx: usize,
    child_indices: &[usize],
) -> Result<HashMap<AttachmentEdgeKey, usize>, String> {
    let parent_name = components
        .get(parent_idx)
        .map(|c| c.name.as_str())
        .unwrap_or("<unknown>");
    let mut out: HashMap<AttachmentEdgeKey, usize> = HashMap::new();
    for &child_idx in child_indices {
        let key = attachment_edge_key_for_child(components, child_idx)?;
        if let Some(existing) = out.insert(key.clone(), child_idx) {
            return Err(format!(
                "copy_component_subtree: ambiguous child mapping under `{}`: duplicate attachment edge key {:?} (children `{}` and `{}`)",
                parent_name,
                key,
                components.get(existing).map(|c| c.name.as_str()).unwrap_or("<unknown>"),
                components.get(child_idx).map(|c| c.name.as_str()).unwrap_or("<unknown>"),
            ));
        }
    }
    Ok(out)
}

fn root_externally_referenced_component_object_ids(draft: &Gen3dDraft) -> HashSet<u128> {
    let mut ids: HashSet<u128> = HashSet::new();
    let Some(root) = draft.root_def() else {
        return ids;
    };

    if let Some(aim) = root.aim.as_ref() {
        ids.extend(aim.components.iter().copied());
    }
    if let Some(attack) = root.attack.as_ref() {
        if let Some(ranged) = attack.ranged.as_ref() {
            ids.insert(ranged.muzzle.object_id);
        }
    }

    ids
}

fn subtree_contains_component_object_id_in_set(
    components: &[Gen3dPlannedComponent],
    children: &[Vec<usize>],
    root_idx: usize,
    object_ids: &HashSet<u128>,
) -> bool {
    if object_ids.is_empty() {
        return false;
    }
    if root_idx >= components.len() || root_idx >= children.len() {
        return false;
    }

    let mut visited = vec![false; components.len()];
    let mut stack = vec![root_idx];
    while let Some(idx) = stack.pop() {
        if idx >= components.len() {
            continue;
        }
        if visited[idx] {
            continue;
        }
        visited[idx] = true;

        let object_id = component_object_id(&components[idx].name);
        if object_ids.contains(&object_id) {
            return true;
        }
        if let Some(child_indices) = children.get(idx) {
            for &child_idx in child_indices {
                stack.push(child_idx);
            }
        }
    }

    false
}

fn map_subtree_pairs(
    components: &[Gen3dPlannedComponent],
    children: &[Vec<usize>],
    source_root_idx: usize,
    target_root_idx: usize,
    skip_component_object_ids: Option<&HashSet<u128>>,
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
        skip_component_object_ids: Option<&HashSet<u128>>,
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

        let source_by_key =
            children_by_attachment_edge_key(components, source_idx, source_children)?;
        let target_by_key =
            children_by_attachment_edge_key(components, target_idx, target_children)?;

        let mut ordered_source: Vec<(AttachmentEdgeKey, usize)> =
            source_by_key.into_iter().collect();
        ordered_source.sort_by(|(a, _), (b, _)| a.cmp(b));
        for (key, s_child) in ordered_source {
            let Some(&t_child) = target_by_key.get(&key) else {
                if skip_component_object_ids.is_some_and(|ids| {
                    subtree_contains_component_object_id_in_set(components, children, s_child, ids)
                }) {
                    continue;
                }
                return Err(format!(
                    "copy_component_subtree: subtree shape mismatch at `{}`: target `{}` missing child attachment {:?} (present in source `{}`)",
                    components[source_idx].name,
                    components[target_idx].name,
                    key,
                    components[source_idx].name,
                ));
            };
            rec(
                components,
                children,
                s_child,
                t_child,
                pairs,
                visiting_source,
                visiting_target,
                skip_component_object_ids,
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
        skip_component_object_ids,
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
    alignment: Gen3dCopyAlignmentMode,
    delta: Transform,
    missing_branch_policy: Gen3dSubtreeCopyMissingBranchPolicy,
) -> Result<Vec<Gen3dCopyComponentOutcome>, String> {
    let referenced_ids = (missing_branch_policy
        == Gen3dSubtreeCopyMissingBranchPolicy::SkipExternallyReferenced)
        .then(|| root_externally_referenced_component_object_ids(draft));
    let referenced_ids = referenced_ids.as_ref();

    ensure_target_subtree_shape(
        components,
        draft,
        source_root_idx,
        target_root_idx,
        anchors_mode,
        referenced_ids,
    )?;

    let children = build_children_map(components);
    let pairs = map_subtree_pairs(
        components,
        &children,
        source_root_idx,
        target_root_idx,
        referenced_ids,
    )?;

    let copied_target_indices = if anchors_mode == Gen3dCopyAnchorsMode::PreserveInterfaceAnchors {
        let mut out: HashSet<usize> = HashSet::new();
        for &(_, target_idx) in pairs.iter() {
            out.insert(target_idx);
        }
        Some(out)
    } else {
        None
    };

    let mut out: Vec<Gen3dCopyComponentOutcome> = Vec::new();
    for (source_idx, target_idx) in pairs {
        let outcome = copy_component_into(
            components.as_mut_slice(),
            draft,
            source_idx,
            target_idx,
            mode,
            anchors_mode,
            alignment,
            delta,
            copied_target_indices.as_ref(),
        )?;
        out.push(outcome);
    }
    Ok(out)
}

pub(super) fn preflight_subtree_copy_pairs(
    components: &[Gen3dPlannedComponent],
    draft: &Gen3dDraft,
    source_root_idx: usize,
    target_root_idx: usize,
    anchors_mode: Gen3dCopyAnchorsMode,
    missing_branch_policy: Gen3dSubtreeCopyMissingBranchPolicy,
) -> Result<Vec<(usize, usize)>, String> {
    let referenced_ids = (missing_branch_policy
        == Gen3dSubtreeCopyMissingBranchPolicy::SkipExternallyReferenced)
        .then(|| root_externally_referenced_component_object_ids(draft));
    let referenced_ids = referenced_ids.as_ref();

    // Run the same structural checks as the real subtree copy path, but on scratch state so we
    // can decide whether to copy or generate before mutating live draft/component data.
    let mut scratch_components = components.to_vec();
    let mut scratch_draft = Gen3dDraft::default();
    ensure_target_subtree_shape(
        &mut scratch_components,
        &mut scratch_draft,
        source_root_idx,
        target_root_idx,
        anchors_mode,
        referenced_ids,
    )?;
    let children = build_children_map(&scratch_components);
    let pairs = map_subtree_pairs(
        &scratch_components,
        &children,
        source_root_idx,
        target_root_idx,
        referenced_ids,
    )?;
    Ok(pairs)
}

fn ensure_target_subtree_shape(
    components: &mut Vec<Gen3dPlannedComponent>,
    draft: &mut Gen3dDraft,
    source_root_idx: usize,
    target_root_idx: usize,
    anchors_mode: Gen3dCopyAnchorsMode,
    skip_component_object_ids: Option<&HashSet<u128>>,
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
            ground_origin_y: None,
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
        source_parent_idx: usize,
        source_child_idx: usize,
        target_parent_idx: usize,
        target_root_name: &str,
        source_children_map: &[Vec<usize>],
        anchors_mode: Gen3dCopyAnchorsMode,
    ) -> Result<usize, String> {
        if components.len() >= GEN3D_MAX_COMPONENTS {
            return Err(format!(
                "copy_component_subtree: cannot expand subtree (max components {GEN3D_MAX_COMPONENTS} reached)"
            ));
        }

        let source = components[source_child_idx].clone();
        let parent_name = components[target_parent_idx].name.clone();
        let mut new_comp = source.clone();
        let proposed_name = format!("{target_root_name}__{}", source.name);
        new_comp.name = allocate_unique_name(components, proposed_name);
        new_comp.display_name = new_comp.name.clone();
        new_comp.pos = Vec3::ZERO;
        new_comp.rot = Quat::IDENTITY;
        new_comp.actual_size = None;
        let Some(mut att) = new_comp.attach_to.clone() else {
            return Err(format!(
                "copy_component_subtree: source component `{}` is missing attach_to; cannot clone under `{}`",
                source.name, components[target_parent_idx].name
            ));
        };
        att.parent = parent_name;
        new_comp.attach_to = Some(att.clone());

        let parent_anchor = att.parent_anchor.as_str();
        if parent_anchor != "origin"
            && !components[target_parent_idx]
                .anchors
                .iter()
                .any(|a| a.name.as_ref() == parent_anchor)
        {
            if anchors_mode == Gen3dCopyAnchorsMode::PreserveTargetAnchors {
                return Err(format!(
                    "copy_component_subtree: cannot attach cloned component `{}` to `{}`: missing parent_anchor `{}`",
                    new_comp.name, components[target_parent_idx].name, parent_anchor
                ));
            }

            let Some(source_parent_anchor) = components.get(source_parent_idx).and_then(|p| {
                p.anchors
                    .iter()
                    .find(|a| a.name.as_ref() == parent_anchor)
                    .cloned()
            }) else {
                return Err(format!(
                    "copy_component_subtree: cannot attach cloned component `{}` to `{}`: missing parent_anchor `{}` (source parent `{}` is missing the same anchor)",
                    new_comp.name,
                    components[target_parent_idx].name,
                    parent_anchor,
                    components
                        .get(source_parent_idx)
                        .map(|c| c.name.as_str())
                        .unwrap_or("<missing>")
                ));
            };

            if let Some(target_parent) = components.get_mut(target_parent_idx) {
                if !target_parent
                    .anchors
                    .iter()
                    .any(|a| a.name.as_ref() == parent_anchor)
                {
                    target_parent.anchors.push(source_parent_anchor.clone());
                }
            }
            ensure_stub_def_exists(draft, &components[target_parent_idx]);
            if let Some(parent_def) = def_for_component(draft, &components[target_parent_idx].name)
            {
                if !parent_def
                    .anchors
                    .iter()
                    .any(|a| a.name.as_ref() == parent_anchor)
                {
                    parent_def.anchors.push(source_parent_anchor);
                }
            }
        }

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

        let new_idx = components.len();
        components.push(new_comp.clone());
        ensure_stub_def_exists(draft, &new_comp);

        let child_id = component_object_id(&new_comp.name);
        let attachment = crate::object::registry::AttachmentDef {
            parent_anchor: att.parent_anchor.clone().into(),
            child_anchor: att.child_anchor.clone().into(),
        };
        let mut part = ObjectPartDef::object_ref(child_id, att.offset).with_attachment(attachment);
        part.animations.extend(att.animations.clone());
        ensure_stub_def_exists(draft, &components[target_parent_idx]);
        if let Some(parent_def) = def_for_component(draft, &components[target_parent_idx].name) {
            parent_def.parts.push(part);
        }

        // Recursively clone descendants.
        for &source_child in source_children_map[source_child_idx].iter() {
            let _ = clone_missing_subtree(
                components,
                draft,
                source_child_idx,
                source_child,
                new_idx,
                target_root_name,
                source_children_map,
                anchors_mode,
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
        anchors_mode: Gen3dCopyAnchorsMode,
        skip_component_object_ids: Option<&HashSet<u128>>,
    ) -> Result<(), String> {
        let children = build_children_map(components);
        let source_children = children.get(source_idx).cloned().unwrap_or_default();
        let target_children = children.get(target_idx).cloned().unwrap_or_default();

        let source_by_key =
            children_by_attachment_edge_key(components.as_slice(), source_idx, &source_children)?;
        let mut target_by_key =
            children_by_attachment_edge_key(components.as_slice(), target_idx, &target_children)?;

        // Clone any missing branches under the target node (allowed even if the target subtree is
        // partially populated), as long as attachment edge keys are unambiguous.
        let mut ordered_source: Vec<(AttachmentEdgeKey, usize)> =
            source_by_key.clone().into_iter().collect();
        ordered_source.sort_by(|(a, _), (b, _)| a.cmp(b));

        let source_children_map = children;
        for (key, s_child) in ordered_source.iter() {
            if target_by_key.contains_key(key) {
                continue;
            }
            if skip_component_object_ids.is_some_and(|ids| {
                subtree_contains_component_object_id_in_set(
                    components.as_slice(),
                    &source_children_map,
                    *s_child,
                    ids,
                )
            }) {
                continue;
            }
            let new_child_idx = clone_missing_subtree(
                components,
                draft,
                source_idx,
                *s_child,
                target_idx,
                target_root_name,
                &source_children_map,
                anchors_mode,
            )?;
            target_by_key.insert(key.clone(), new_child_idx);
        }

        // Recurse into matched children (including newly cloned ones).
        for (key, s_child) in ordered_source {
            let Some(&t_child) = target_by_key.get(&key) else {
                if skip_component_object_ids.is_some_and(|ids| {
                    subtree_contains_component_object_id_in_set(
                        components.as_slice(),
                        &source_children_map,
                        s_child,
                        ids,
                    )
                }) {
                    continue;
                }
                return Err(format!(
                    "copy_component_subtree: internal error: missing cloned child for attachment key {:?} under `{}`",
                    key, components[target_idx].name
                ));
            };
            rec(
                components,
                draft,
                s_child,
                t_child,
                target_root_name,
                anchors_mode,
                skip_component_object_ids,
            )?;
        }
        Ok(())
    }

    rec(
        components,
        draft,
        source_root_idx,
        target_root_idx,
        &target_root_name,
        anchors_mode,
        skip_component_object_ids,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::registry::{ColliderProfile, ObjectInteraction};

    fn anchor_named(name: &str, pos: Vec3, forward: Vec3, up: Vec3) -> AnchorDef {
        let rot = super::super::convert::plan_rotation_from_forward_up_lossy(forward, Some(up));
        AnchorDef {
            name: name.to_string().into(),
            transform: Transform {
                translation: pos,
                rotation: rot,
                ..default()
            },
        }
    }

    fn anchor_named_forward(name: &str, forward: Vec3) -> AnchorDef {
        let rot =
            super::super::convert::plan_rotation_from_forward_up_lossy(forward, Some(Vec3::Y));
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
            ground_origin_y: None,
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
            Gen3dCopyAlignmentMode::Rotation,
            Transform::IDENTITY,
            None,
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
            Gen3dCopyAlignmentMode::Rotation,
            Transform::IDENTITY,
            None,
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
            Gen3dCopyAlignmentMode::Rotation,
            Transform::IDENTITY,
            Gen3dSubtreeCopyMissingBranchPolicy::CloneAllMissing,
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
    fn copy_alignment_accounts_for_attachment_offset_rotation() {
        // Regression test for a common Gen3D plan pattern:
        // - child anchors are flipped to satisfy the engine's join hemisphere guard
        // - attach_to.offset rotation compensates so the modeled geometry still points outward
        //
        // When reusing geometry, alignment must consider BOTH the child anchor and the offset, or
        // we can accidentally bake an extra 180° into internal anchors (double-flip).
        let mut components = vec![stub_component("arm_0"), stub_component("arm_2")];
        components[0].actual_size = Some(Vec3::ONE);

        components[0].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "body".into(),
            parent_anchor: "arm_mount_front".into(),
            child_anchor: "body_mount".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });
        components[1].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "body".into(),
            parent_anchor: "arm_mount_back".into(),
            child_anchor: "body_mount".into(),
            offset: Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
            joint: None,
            animations: Vec::new(),
        });

        let arm_0_id = component_object_id("arm_0");
        let arm_2_id = component_object_id("arm_2");

        let mut arm_0_def = stub_def(arm_0_id, "arm_0");
        arm_0_def.anchors = vec![
            AnchorDef {
                name: "origin".into(),
                transform: Transform::IDENTITY,
            },
            anchor_named("body_mount", Vec3::ZERO, Vec3::Z, Vec3::Y),
            anchor_named(
                "rotor_mount",
                Vec3::new(0.0, 0.02, 0.22),
                Vec3::Y,
                Vec3::Z,
            ),
        ];

        let mut arm_2_def = stub_def(arm_2_id, "arm_2");
        arm_2_def.anchors = vec![
            AnchorDef {
                name: "origin".into(),
                transform: Transform::IDENTITY,
            },
            // The mount anchor is flipped (dot-guard), but the offset rotates back in JOIN.
            anchor_named("body_mount", Vec3::ZERO, Vec3::NEG_Z, Vec3::Y),
            // Include the internal anchor so PreserveInterfaceAnchors will overwrite it.
            anchor_named(
                "rotor_mount",
                Vec3::new(0.0, 0.02, 0.22),
                Vec3::Y,
                Vec3::Z,
            ),
        ];

        let mut draft = Gen3dDraft::default();
        draft.defs = vec![arm_0_def, arm_2_def];

        copy_component_into(
            &mut components,
            &mut draft,
            0,
            1,
            Gen3dCopyMode::Detached,
            Gen3dCopyAnchorsMode::PreserveInterfaceAnchors,
            Gen3dCopyAlignmentMode::Rotation,
            Transform::IDENTITY,
            None,
        )
        .expect("copy ok");

        let arm_2_after = draft.defs.iter().find(|d| d.object_id == arm_2_id).unwrap();
        let rotor_mount = arm_2_after
            .anchors
            .iter()
            .find(|a| a.name.as_ref() == "rotor_mount")
            .expect("rotor_mount anchor present");
        assert!(
            (rotor_mount.transform.translation.z - 0.22).abs() < 1e-4,
            "expected rotor_mount to stay at +Z tip, got {:?}",
            rotor_mount.transform.translation
        );
    }

    #[test]
    fn subtree_copy_allows_extra_target_child_attachments() {
        let mut components = vec![
            stub_component("hand_l"),
            stub_component("finger_l"),
            stub_component("hand_r"),
            stub_component("finger_r"),
            stub_component("grip_socket"),
        ];

        components[0]
            .anchors
            .push(anchor_named_forward("mount", Vec3::Z));
        components[0]
            .anchors
            .push(anchor_named_forward("grip", Vec3::X));
        components[1]
            .anchors
            .push(anchor_named_forward("socket", Vec3::Z));
        components[2]
            .anchors
            .push(anchor_named_forward("mount", Vec3::Z));
        components[2]
            .anchors
            .push(anchor_named_forward("grip", Vec3::X));
        components[3]
            .anchors
            .push(anchor_named_forward("socket", Vec3::Z));
        components[4]
            .anchors
            .push(anchor_named_forward("grip_socket", Vec3::Z));

        components[1].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "hand_l".into(),
            parent_anchor: "mount".into(),
            child_anchor: "socket".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });
        components[3].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "hand_r".into(),
            parent_anchor: "mount".into(),
            child_anchor: "socket".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });
        components[4].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "hand_r".into(),
            parent_anchor: "grip".into(),
            child_anchor: "grip_socket".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });

        // Source subtree is generated.
        components[0].actual_size = Some(Vec3::ONE);
        components[1].actual_size = Some(Vec3::ONE);

        let hand_l_id = component_object_id("hand_l");
        let finger_l_id = component_object_id("finger_l");
        let hand_r_id = component_object_id("hand_r");
        let finger_r_id = component_object_id("finger_r");
        let grip_socket_id = component_object_id("grip_socket");

        let mut hand_l_def = stub_def(hand_l_id, "hand_l");
        hand_l_def.anchors = vec![
            AnchorDef {
                name: "origin".into(),
                transform: Transform::IDENTITY,
            },
            anchor_named_forward("mount", Vec3::Z),
            anchor_named_forward("grip", Vec3::NEG_X),
        ];

        let mut finger_l_def = stub_def(finger_l_id, "finger_l");
        finger_l_def
            .anchors
            .push(anchor_named_forward("socket", Vec3::Z));

        let mut hand_r_def = stub_def(hand_r_id, "hand_r");
        let grip_before = anchor_named_forward("grip", Vec3::X);
        hand_r_def.anchors = vec![
            AnchorDef {
                name: "origin".into(),
                transform: Transform::IDENTITY,
            },
            anchor_named_forward("mount", Vec3::Z),
            grip_before.clone(),
        ];
        hand_r_def.parts = vec![
            ObjectPartDef::primitive(
                crate::object::registry::PrimitiveVisualDef::Primitive {
                    mesh: crate::object::registry::MeshKey::UnitCube,
                    params: None,
                    color: Color::WHITE,
                    unlit: false,
                },
                Transform::from_scale(Vec3::splat(1.0)),
            ),
            ObjectPartDef::object_ref(finger_r_id, Transform::IDENTITY).with_attachment(
                crate::object::registry::AttachmentDef {
                    parent_anchor: "mount".into(),
                    child_anchor: "socket".into(),
                },
            ),
            ObjectPartDef::object_ref(grip_socket_id, Transform::IDENTITY).with_attachment(
                crate::object::registry::AttachmentDef {
                    parent_anchor: "grip".into(),
                    child_anchor: "grip_socket".into(),
                },
            ),
        ];

        let mut finger_r_def = stub_def(finger_r_id, "finger_r");
        finger_r_def
            .anchors
            .push(anchor_named_forward("socket", Vec3::Z));
        let mut grip_socket_def = stub_def(grip_socket_id, "grip_socket");
        grip_socket_def
            .anchors
            .push(anchor_named_forward("grip_socket", Vec3::Z));

        let mut draft = Gen3dDraft::default();
        draft.defs = vec![
            hand_l_def,
            finger_l_def,
            hand_r_def,
            finger_r_def,
            grip_socket_def,
        ];

        copy_component_subtree_into(
            &mut components,
            &mut draft,
            0,
            2,
            Gen3dCopyMode::Detached,
            Gen3dCopyAnchorsMode::PreserveInterfaceAnchors,
            Gen3dCopyAlignmentMode::Rotation,
            Transform::IDENTITY,
            Gen3dSubtreeCopyMissingBranchPolicy::CloneAllMissing,
        )
        .expect("subtree copy ok");

        let hand_r_after = draft
            .defs
            .iter()
            .find(|d| d.object_id == hand_r_id)
            .unwrap();
        assert!(
            hand_r_after
                .parts
                .iter()
                .any(|p| matches!(p.kind, ObjectPartKind::ObjectRef { object_id } if object_id == grip_socket_id)
                    && p.attachment.as_ref().is_some_and(|att| att.parent_anchor.as_ref() == "grip" && att.child_anchor.as_ref() == "grip_socket")),
            "expected subtree copy to preserve extra target attachment ref to grip_socket"
        );

        let grip_after =
            anchor_transform_from_defs(&hand_r_after.anchors, "grip").expect("grip anchor");
        let forward = grip_after.rotation * Vec3::Z;
        assert!(
            forward.dot(Vec3::X) > 0.99,
            "expected preserve_interfaces subtree copy to preserve target grip anchor, got {forward:?}"
        );
        assert_eq!(
            grip_after, grip_before.transform,
            "expected target grip anchor transform to be preserved"
        );
    }

    #[test]
    fn subtree_copy_skip_policy_avoids_cloning_externally_referenced_subtrees() {
        use crate::object::registry::{
            AnchorRef, RangedAttackProfile, UnitAttackKind, UnitAttackProfile,
        };

        fn setup() -> (Vec<Gen3dPlannedComponent>, Gen3dDraft, u128) {
            let mut components = vec![
                stub_component("hand_l"),
                stub_component("hand_r"),
                stub_component("gun_core"),
                stub_component("gun_barrel"),
            ];

            components[0]
                .anchors
                .push(anchor_named_forward("grip", Vec3::X));
            components[1]
                .anchors
                .push(anchor_named_forward("grip", Vec3::X));
            components[2]
                .anchors
                .push(anchor_named_forward("mount", Vec3::Z));
            components[3]
                .anchors
                .push(anchor_named_forward("mount", Vec3::Z));
            components[3]
                .anchors
                .push(anchor_named_forward("muzzle", Vec3::Z));

            components[2].attach_to = Some(super::super::Gen3dPlannedAttachment {
                parent: "hand_r".into(),
                parent_anchor: "grip".into(),
                child_anchor: "mount".into(),
                offset: Transform::IDENTITY,
                joint: None,
                animations: Vec::new(),
            });
            components[3].attach_to = Some(super::super::Gen3dPlannedAttachment {
                parent: "gun_core".into(),
                parent_anchor: "mount".into(),
                child_anchor: "mount".into(),
                offset: Transform::IDENTITY,
                joint: None,
                animations: Vec::new(),
            });

            // Source subtree is generated.
            components[1].actual_size = Some(Vec3::ONE);
            components[2].actual_size = Some(Vec3::ONE);
            components[3].actual_size = Some(Vec3::ONE);

            let root_id = super::super::super::gen3d_draft_object_id();
            let projectile_id = super::super::super::gen3d_draft_projectile_object_id();
            let hand_l_id = component_object_id("hand_l");
            let gun_barrel_id = component_object_id("gun_barrel");

            let mut root_def = stub_def(root_id, "root");
            root_def.attack = Some(UnitAttackProfile {
                kind: UnitAttackKind::RangedProjectile,
                cooldown_secs: 0.5,
                damage: 1,
                anim_window_secs: 0.35,
                melee: None,
                ranged: Some(RangedAttackProfile {
                    projectile_prefab: projectile_id,
                    muzzle: AnchorRef {
                        object_id: gun_barrel_id,
                        anchor: "muzzle".into(),
                    },
                }),
            });

            let mut hand_l_def = stub_def(hand_l_id, "hand_l");
            hand_l_def.anchors = components[0].anchors.clone();
            hand_l_def.parts.clear();

            let mut hand_r_def = stub_def(component_object_id("hand_r"), "hand_r");
            hand_r_def.anchors = components[1].anchors.clone();

            let mut gun_core_def = stub_def(component_object_id("gun_core"), "gun_core");
            gun_core_def.anchors = components[2].anchors.clone();

            let mut gun_barrel_def = stub_def(gun_barrel_id, "gun_barrel");
            gun_barrel_def.anchors = components[3].anchors.clone();

            let mut draft = Gen3dDraft::default();
            draft.defs = vec![
                root_def,
                hand_l_def,
                hand_r_def,
                gun_core_def,
                gun_barrel_def,
            ];

            (components, draft, hand_l_id)
        }

        // CloneAllMissing keeps explicit tool behavior: clone missing branches.
        let (mut components, mut draft, _hand_l_id) = setup();
        let before_len = components.len();
        copy_component_subtree_into(
            &mut components,
            &mut draft,
            1,
            0,
            Gen3dCopyMode::Detached,
            Gen3dCopyAnchorsMode::PreserveTargetAnchors,
            Gen3dCopyAlignmentMode::Rotation,
            Transform::IDENTITY,
            Gen3dSubtreeCopyMissingBranchPolicy::CloneAllMissing,
        )
        .expect("subtree copy ok");
        assert!(
            components.len() > before_len,
            "expected CloneAllMissing subtree copy to clone missing branches"
        );
        assert!(
            components.iter().any(|c| c.name.starts_with("hand_l__")),
            "expected cloned `hand_l__*` components to exist"
        );

        // SkipExternallyReferenced avoids cloning branches that contain externally referenced ids.
        let (mut components, mut draft, hand_l_id) = setup();
        let before_len = components.len();
        copy_component_subtree_into(
            &mut components,
            &mut draft,
            1,
            0,
            Gen3dCopyMode::Detached,
            Gen3dCopyAnchorsMode::PreserveTargetAnchors,
            Gen3dCopyAlignmentMode::Rotation,
            Transform::IDENTITY,
            Gen3dSubtreeCopyMissingBranchPolicy::SkipExternallyReferenced,
        )
        .expect("subtree copy ok");

        assert_eq!(
            components.len(),
            before_len,
            "expected SkipExternallyReferenced subtree copy to not clone externally referenced branches"
        );
        assert!(
            !components.iter().any(|c| c.name.starts_with("hand_l__")),
            "expected no cloned `hand_l__*` components when skipping externally referenced branches"
        );

        let hand_l_after = draft
            .defs
            .iter()
            .find(|d| d.object_id == hand_l_id)
            .unwrap();
        assert!(
            hand_l_after
                .parts
                .iter()
                .any(|p| matches!(p.kind, ObjectPartKind::Primitive { .. })),
            "expected subtree copy to materialize primitives into hand_l"
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
            ground_origin_y: None,
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
            ground_origin_y: None,
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
            Gen3dCopyAlignmentMode::Rotation,
            Transform::IDENTITY,
            None,
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
    fn mirror_mount_x_alignment_flips_geometry_and_non_interface_anchors() {
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
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: vec![
                anchor_named_forward("mount", Vec3::Z),
                anchor_named("tip", Vec3::X, Vec3::Z, Vec3::Y),
            ],
            parts: vec![ObjectPartDef::primitive(
                crate::object::registry::PrimitiveVisualDef::Primitive {
                    mesh: crate::object::registry::MeshKey::UnitCube,
                    params: None,
                    color: Color::WHITE,
                    unlit: false,
                },
                Transform::from_translation(Vec3::new(0.25, 0.0, 0.0)),
            )],
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        };

        let target_mount_before = anchor_named_forward("mount", Vec3::Z).transform;
        let target_def = ObjectDef {
            object_id: b_id,
            label: "b".into(),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: vec![
                AnchorDef {
                    name: "mount".into(),
                    transform: target_mount_before,
                },
                anchor_named("tip", Vec3::ZERO, Vec3::Z, Vec3::Y),
            ],
            parts: Vec::new(),
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        };

        let mut draft = Gen3dDraft::default();
        draft.defs = vec![source_def, target_def];

        copy_component_into(
            &mut components,
            &mut draft,
            0,
            1,
            Gen3dCopyMode::Detached,
            Gen3dCopyAnchorsMode::PreserveInterfaceAnchors,
            Gen3dCopyAlignmentMode::MirrorMountX,
            Transform::IDENTITY,
            None,
        )
        .expect("mirror copy ok");

        let b_def_after = draft.defs.iter().find(|d| d.object_id == b_id).unwrap();

        let mount_after =
            anchor_transform_from_defs(&b_def_after.anchors, "mount").expect("mount anchor");
        assert_eq!(
            mount_after, target_mount_before,
            "expected mount anchor to be preserved under preserve_interfaces"
        );

        let tip_after =
            anchor_transform_from_defs(&b_def_after.anchors, "tip").expect("tip anchor");
        let dp_tip = (tip_after.translation - Vec3::new(-1.0, 0.0, 0.0)).length();
        assert!(
            dp_tip < 1e-4,
            "expected tip anchor to be mirrored across mount-local +X; delta={dp_tip} tip={:?}",
            tip_after.translation
        );

        let part = b_def_after
            .parts
            .iter()
            .find(|p| matches!(p.kind, ObjectPartKind::Primitive { .. }))
            .expect("copied primitive");
        let det = part.transform.scale.x * part.transform.scale.y * part.transform.scale.z;
        assert!(
            det.is_finite() && det < 0.0,
            "expected mirrored copy to have negative scale determinant, got scale={:?} det={det}",
            part.transform.scale
        );
        assert!(
            part.transform.translation.x < 0.0,
            "expected mirrored part translation.x to flip sign, got {:?}",
            part.transform.translation
        );
    }

    #[test]
    fn preserve_interfaces_preserves_mount_but_overwrites_other_anchors() {
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
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: vec![
                anchor_named("mount", Vec3::new(1.775, 0.0, 0.3), Vec3::NEG_X, Vec3::Y),
                anchor_named(
                    "tip_mount",
                    Vec3::new(-1.425, 0.0, 0.5),
                    Vec3::NEG_X,
                    Vec3::Y,
                ),
            ],
            parts: vec![ObjectPartDef::primitive(
                crate::object::registry::PrimitiveVisualDef::Primitive {
                    mesh: crate::object::registry::MeshKey::UnitCube,
                    params: None,
                    color: Color::WHITE,
                    unlit: false,
                },
                Transform::IDENTITY,
            )],
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        };

        let target_mount_before = anchor_named("mount", Vec3::ZERO, Vec3::X, Vec3::Y).transform;
        let target_def = ObjectDef {
            object_id: b_id,
            label: "b".into(),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: vec![
                AnchorDef {
                    name: "mount".into(),
                    transform: target_mount_before,
                },
                anchor_named("tip_mount", Vec3::new(3.2, 0.0, 0.2), Vec3::X, Vec3::Y),
            ],
            parts: Vec::new(),
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        };

        let source_mount =
            anchor_transform_from_defs(&source_def.anchors, "mount").expect("source mount");
        let inv_source_mount = source_mount.to_matrix().inverse();
        assert!(
            inv_source_mount.is_finite(),
            "expected mount to be invertible"
        );

        let rot_mat = target_mount_before.to_matrix() * inv_source_mount;

        let source_tip_mount =
            anchor_transform_from_defs(&source_def.anchors, "tip_mount").expect("source tip_mount");
        let expected_tip_mount_translation = transform_point(rot_mat, source_tip_mount.translation);

        let mut draft = Gen3dDraft::default();
        draft.defs = vec![source_def, target_def];

        copy_component_into(
            &mut components,
            &mut draft,
            0,
            1,
            Gen3dCopyMode::Detached,
            Gen3dCopyAnchorsMode::PreserveInterfaceAnchors,
            Gen3dCopyAlignmentMode::Rotation,
            Transform::IDENTITY,
            None,
        )
        .expect("copy ok");

        let b_def_after = draft.defs.iter().find(|d| d.object_id == b_id).unwrap();
        let mount_after =
            anchor_transform_from_defs(&b_def_after.anchors, "mount").expect("mount anchor");
        assert_eq!(
            mount_after, target_mount_before,
            "expected mount anchor to be preserved under preserve_interfaces"
        );

        let tip_after = anchor_transform_from_defs(&b_def_after.anchors, "tip_mount")
            .expect("tip_mount anchor");
        let dp = (tip_after.translation - expected_tip_mount_translation).length();
        assert!(
            dp < 1e-4,
            "expected tip_mount to be overwritten from source under preserve_interfaces; delta={dp}"
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
            Gen3dCopyAlignmentMode::Rotation,
            Transform::IDENTITY,
            Gen3dSubtreeCopyMissingBranchPolicy::CloneAllMissing,
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

    #[test]
    fn subtree_copy_hydrates_missing_internal_parent_anchors_on_targets() {
        // Repro for the octopus: the plan declares a reused subtree where the source parent has an
        // internal anchor (e.g. `next`) required to attach descendants, but the target parent does
        // not define that anchor in the plan. Subtree copy should hydrate the missing parent anchor
        // deterministically so the subtree can be expanded and copied.
        let mut components = vec![
            stub_component("body"),
            stub_component("tentacle_a_base"),
            stub_component("tentacle_a_mid"),
            stub_component("tentacle_b_base"),
        ];
        components[0].anchors = vec![
            anchor_named_forward("mount_a", Vec3::Z),
            anchor_named_forward("mount_b", Vec3::Z),
        ];
        components[1].anchors = vec![
            anchor_named_forward("mount", Vec3::Z),
            anchor_named_forward("next", Vec3::Z),
        ];
        components[2].anchors = vec![anchor_named_forward("mount", Vec3::Z)];
        // Target base is missing `next`.
        components[3].anchors = vec![anchor_named_forward("mount", Vec3::Z)];

        components[1].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "body".into(),
            parent_anchor: "mount_a".into(),
            child_anchor: "mount".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });
        components[2].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "tentacle_a_base".into(),
            parent_anchor: "next".into(),
            child_anchor: "mount".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });
        components[3].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "body".into(),
            parent_anchor: "mount_b".into(),
            child_anchor: "mount".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });

        // Mark source chain as generated.
        components[1].actual_size = Some(Vec3::ONE);
        components[2].actual_size = Some(Vec3::ONE);

        let body_id = component_object_id("body");
        let a_base_id = component_object_id("tentacle_a_base");
        let a_mid_id = component_object_id("tentacle_a_mid");
        let b_base_id = component_object_id("tentacle_b_base");

        let mut a_base_def = stub_def(a_base_id, "tentacle_a_base");
        a_base_def.anchors = vec![
            anchor_named_forward("mount", Vec3::Z),
            anchor_named_forward("next", Vec3::Z),
        ];
        let mut a_mid_def = stub_def(a_mid_id, "tentacle_a_mid");
        a_mid_def.anchors = vec![anchor_named_forward("mount", Vec3::Z)];
        let mut b_base_def = stub_def(b_base_id, "tentacle_b_base");
        b_base_def.anchors = vec![anchor_named_forward("mount", Vec3::Z)];

        let mut draft = Gen3dDraft::default();
        draft.defs = vec![
            {
                let mut def = stub_def(body_id, "body");
                def.anchors = vec![
                    anchor_named_forward("mount_a", Vec3::Z),
                    anchor_named_forward("mount_b", Vec3::Z),
                ];
                def
            },
            a_base_def,
            a_mid_def,
            b_base_def,
        ];

        copy_component_subtree_into(
            &mut components,
            &mut draft,
            1,
            3,
            Gen3dCopyMode::Detached,
            Gen3dCopyAnchorsMode::PreserveInterfaceAnchors,
            Gen3dCopyAlignmentMode::Rotation,
            Transform::IDENTITY,
            Gen3dSubtreeCopyMissingBranchPolicy::CloneAllMissing,
        )
        .expect("subtree copy ok");

        assert!(
            components
                .iter()
                .any(|c| c.name.as_str() == "tentacle_b_base__tentacle_a_mid"),
            "expected subtree copy to expand missing descendants under the target root"
        );

        let b_base_def_after = draft
            .defs
            .iter()
            .find(|d| d.object_id == b_base_id)
            .unwrap();
        assert!(
            b_base_def_after
                .anchors
                .iter()
                .any(|a| a.name.as_ref() == "next"),
            "expected subtree copy to hydrate missing internal anchor `next` on the target root"
        );

        let mid_id = component_object_id("tentacle_b_base__tentacle_a_mid");
        assert!(
            b_base_def_after.parts.iter().any(|p| {
                matches!(p.kind, ObjectPartKind::ObjectRef { object_id } if object_id == mid_id)
                    && p.attachment.as_ref().is_some_and(|att| {
                        att.parent_anchor.as_ref() == "next" && att.child_anchor.as_ref() == "mount"
                    })
            }),
            "expected subtree copy to attach the expanded mid segment under `next`"
        );
    }

    #[test]
    fn subtree_copy_preserve_target_stays_strict_when_parent_anchor_is_missing() {
        let mut components = vec![
            stub_component("body"),
            stub_component("tentacle_a_base"),
            stub_component("tentacle_a_mid"),
            stub_component("tentacle_b_base"),
        ];
        components[0].anchors = vec![
            anchor_named_forward("mount_a", Vec3::Z),
            anchor_named_forward("mount_b", Vec3::Z),
        ];
        components[1].anchors = vec![
            anchor_named_forward("mount", Vec3::Z),
            anchor_named_forward("next", Vec3::Z),
        ];
        components[2].anchors = vec![anchor_named_forward("mount", Vec3::Z)];
        components[3].anchors = vec![anchor_named_forward("mount", Vec3::Z)];

        components[1].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "body".into(),
            parent_anchor: "mount_a".into(),
            child_anchor: "mount".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });
        components[2].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "tentacle_a_base".into(),
            parent_anchor: "next".into(),
            child_anchor: "mount".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });
        components[3].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "body".into(),
            parent_anchor: "mount_b".into(),
            child_anchor: "mount".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });

        // Mark source chain as generated.
        components[1].actual_size = Some(Vec3::ONE);
        components[2].actual_size = Some(Vec3::ONE);

        let body_id = component_object_id("body");
        let a_base_id = component_object_id("tentacle_a_base");
        let a_mid_id = component_object_id("tentacle_a_mid");
        let b_base_id = component_object_id("tentacle_b_base");

        let mut a_base_def = stub_def(a_base_id, "tentacle_a_base");
        a_base_def.anchors = vec![
            anchor_named_forward("mount", Vec3::Z),
            anchor_named_forward("next", Vec3::Z),
        ];
        let mut a_mid_def = stub_def(a_mid_id, "tentacle_a_mid");
        a_mid_def.anchors = vec![anchor_named_forward("mount", Vec3::Z)];
        let mut b_base_def = stub_def(b_base_id, "tentacle_b_base");
        b_base_def.anchors = vec![anchor_named_forward("mount", Vec3::Z)];

        let mut draft = Gen3dDraft::default();
        draft.defs = vec![
            {
                let mut def = stub_def(body_id, "body");
                def.anchors = vec![
                    anchor_named_forward("mount_a", Vec3::Z),
                    anchor_named_forward("mount_b", Vec3::Z),
                ];
                def
            },
            a_base_def,
            a_mid_def,
            b_base_def,
        ];

        let before_components = components.len();
        let before_defs = draft.defs.len();
        let err = copy_component_subtree_into(
            &mut components,
            &mut draft,
            1,
            3,
            Gen3dCopyMode::Detached,
            Gen3dCopyAnchorsMode::PreserveTargetAnchors,
            Gen3dCopyAlignmentMode::Rotation,
            Transform::IDENTITY,
            Gen3dSubtreeCopyMissingBranchPolicy::CloneAllMissing,
        )
        .expect_err("expected strict preserve_target to fail");
        assert!(
            err.contains("missing parent_anchor") && err.contains("next"),
            "expected missing parent_anchor error, got {err:?}"
        );
        assert_eq!(
            components.len(),
            before_components,
            "expected preserve_target failure to not expand components"
        );
        assert_eq!(
            draft.defs.len(),
            before_defs,
            "expected preserve_target failure to not add new defs"
        );
        assert!(
            !components
                .iter()
                .any(|c| c.name.as_str() == "tentacle_b_base__tentacle_a_mid"),
            "expected preserve_target failure to not create partial descendant components"
        );
    }

    #[test]
    fn preserve_interfaces_subtree_does_not_preserve_internal_child_attachment_anchors() {
        // Repro for the wing case: a subtree copy that requires a mount alignment delta should
        // also rotate/translate internal child-attachment anchors. `preserve_target` keeps those
        // internal anchors unchanged, which can drift attachments. `preserve_interfaces` should
        // overwrite internal anchors while preserving external interfaces.
        let mut components = vec![
            stub_component("body"),
            stub_component("wing_root_L"),
            stub_component("wing_tip_L"),
            stub_component("wing_root_R"),
            stub_component("wing_tip_R"),
        ];

        // Attach roots to body.
        components[1].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "body".into(),
            parent_anchor: "wing_mount_L".into(),
            child_anchor: "shoulder".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });
        components[3].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "body".into(),
            parent_anchor: "wing_mount_R".into(),
            child_anchor: "shoulder".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });

        // Attach tips to roots via tip_mount/root.
        components[2].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "wing_root_L".into(),
            parent_anchor: "tip_mount".into(),
            child_anchor: "root".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });
        components[4].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "wing_root_R".into(),
            parent_anchor: "tip_mount".into(),
            child_anchor: "root".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });

        // Mark source subtree as generated.
        components[1].actual_size = Some(Vec3::ONE);
        components[2].actual_size = Some(Vec3::ONE);

        let body_id = component_object_id("body");
        let wing_root_l_id = component_object_id("wing_root_L");
        let wing_tip_l_id = component_object_id("wing_tip_L");
        let wing_root_r_id = component_object_id("wing_root_R");
        let wing_tip_r_id = component_object_id("wing_tip_R");

        let mut draft = Gen3dDraft::default();
        draft.defs = vec![
            stub_def(body_id, "body"),
            ObjectDef {
                anchors: vec![
                    // Mount +X for the LEFT wing in local space; copy will rotate to +X for right.
                    anchor_named("shoulder", Vec3::new(1.775, 0.0, 0.3), Vec3::NEG_X, Vec3::Y),
                    anchor_named(
                        "tip_mount",
                        Vec3::new(-1.425, 0.0, 0.5),
                        Vec3::NEG_X,
                        Vec3::Y,
                    ),
                ],
                ..stub_def(wing_root_l_id, "wing_root_L")
            },
            ObjectDef {
                anchors: vec![anchor_named("root", Vec3::ZERO, Vec3::NEG_X, Vec3::Y)],
                ..stub_def(wing_tip_l_id, "wing_tip_L")
            },
            ObjectDef {
                anchors: vec![
                    // Target mount interface must remain stable.
                    anchor_named("shoulder", Vec3::ZERO, Vec3::X, Vec3::Y),
                    // This is intentionally the "wrong" Z sign for a 180-degree copy; it should be
                    // overwritten when the attached child is internal to the subtree.
                    anchor_named("tip_mount", Vec3::new(3.2, 0.0, 0.2), Vec3::X, Vec3::Y),
                ],
                parts: Vec::new(),
                ..stub_def(wing_root_r_id, "wing_root_R")
            },
            ObjectDef {
                anchors: vec![anchor_named("root", Vec3::ZERO, Vec3::X, Vec3::Y)],
                parts: Vec::new(),
                ..stub_def(wing_tip_r_id, "wing_tip_R")
            },
        ];

        let target_mount_before = anchor_transform_from_defs(
            &draft
                .defs
                .iter()
                .find(|d| d.object_id == wing_root_r_id)
                .unwrap()
                .anchors,
            "shoulder",
        )
        .expect("target shoulder");
        copy_component_subtree_into(
            &mut components,
            &mut draft,
            1,
            3,
            Gen3dCopyMode::Detached,
            Gen3dCopyAnchorsMode::PreserveInterfaceAnchors,
            Gen3dCopyAlignmentMode::Rotation,
            Transform::IDENTITY,
            Gen3dSubtreeCopyMissingBranchPolicy::CloneAllMissing,
        )
        .expect("subtree copy ok");

        let wing_root_r_after = draft
            .defs
            .iter()
            .find(|d| d.object_id == wing_root_r_id)
            .unwrap();
        let mount_after =
            anchor_transform_from_defs(&wing_root_r_after.anchors, "shoulder").expect("shoulder");
        assert_eq!(
            mount_after, target_mount_before,
            "expected subtree copy to preserve the target mount interface"
        );

        let source_root_def = draft
            .defs
            .iter()
            .find(|d| d.object_id == wing_root_l_id)
            .unwrap();
        let source_mount =
            anchor_transform_from_defs(&source_root_def.anchors, "shoulder").expect("shoulder");
        let inv_source_mount = source_mount.to_matrix().inverse();
        assert!(
            inv_source_mount.is_finite(),
            "expected source mount to be invertible"
        );

        let rot_mat = target_mount_before.to_matrix() * inv_source_mount;

        let source_tip_mount =
            anchor_transform_from_defs(&source_root_def.anchors, "tip_mount").expect("tip_mount");
        let expected_tip_mount_translation = transform_point(rot_mat, source_tip_mount.translation);
        let tip_after =
            anchor_transform_from_defs(&wing_root_r_after.anchors, "tip_mount").expect("tip_mount");
        let dp = (tip_after.translation - expected_tip_mount_translation).length();
        assert!(
            dp < 1e-4,
            "expected subtree copy to overwrite internal child-attachment anchors; delta={dp}"
        );
    }

    #[test]
    fn preserve_interfaces_prefers_mirrored_alignment_when_target_layout_is_mirrored() {
        // Repro for the dragon wing: the target is a mirror of the source across the mount
        // interface (left/right), so the copy should choose a mirrored alignment rather than a
        // 180-degree rotation (which would flip -Z to +Z for internal anchors).
        let mut components = vec![stub_component("wing_root_L"), stub_component("wing_root_R")];
        components[0].actual_size = Some(Vec3::ONE);
        components[0].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "torso".into(),
            parent_anchor: "wing_mount_L".into(),
            child_anchor: "torso_mount".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });
        components[1].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "torso".into(),
            parent_anchor: "wing_mount_R".into(),
            child_anchor: "torso_mount".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });

        let wing_root_l_id = component_object_id("wing_root_L");
        let wing_root_r_id = component_object_id("wing_root_R");

        let mut draft = Gen3dDraft::default();
        draft.defs = vec![
            ObjectDef {
                anchors: vec![
                    anchor_named(
                        "torso_mount",
                        Vec3::new(0.55, 0.0, 0.0),
                        Vec3::NEG_X,
                        Vec3::Y,
                    ),
                    anchor_named(
                        "fingers_mount",
                        Vec3::new(-0.2, 0.0, -0.55),
                        Vec3::NEG_Z,
                        Vec3::Y,
                    ),
                ],
                ..stub_def(wing_root_l_id, "wing_root_L")
            },
            ObjectDef {
                anchors: vec![
                    anchor_named("torso_mount", Vec3::new(-0.55, 0.0, 0.0), Vec3::X, Vec3::Y),
                    anchor_named(
                        "fingers_mount",
                        Vec3::new(0.2, 0.0, -0.55),
                        Vec3::NEG_Z,
                        Vec3::Y,
                    ),
                ],
                parts: Vec::new(),
                ..stub_def(wing_root_r_id, "wing_root_R")
            },
        ];

        copy_component_into(
            &mut components,
            &mut draft,
            0,
            1,
            Gen3dCopyMode::Detached,
            Gen3dCopyAnchorsMode::PreserveInterfaceAnchors,
            Gen3dCopyAlignmentMode::MirrorMountX,
            Transform::IDENTITY,
            None,
        )
        .expect("copy ok");

        let wing_root_r_after = draft
            .defs
            .iter()
            .find(|d| d.object_id == wing_root_r_id)
            .unwrap();
        let fingers_mount = anchor_transform_from_defs(&wing_root_r_after.anchors, "fingers_mount")
            .expect("fingers_mount");
        let dp = (fingers_mount.translation - Vec3::new(0.2, 0.0, -0.55)).length();
        assert!(
            dp < 1e-4,
            "expected mirrored copy to preserve the target's -Z layout; delta={dp}"
        );
        let forward = (fingers_mount.rotation * Vec3::Z).normalize();
        assert!(
            forward.dot(Vec3::NEG_Z) > 1.0 - 1e-4,
            "expected mirrored copy to keep fingers_mount forward=-Z; got {forward:?}"
        );
    }

    #[test]
    fn preserve_interfaces_can_choose_mirrored_alignment_with_only_mount_anchor() {
        // If the only shared anchor is the mount interface, the proper-rotation and mirrored
        // candidates tie on anchor alignment. The copy should still pick the mirrored candidate
        // when the target's layout around the mount is mirrored (as in L/R wings).
        let mut components = vec![stub_component("wing_left"), stub_component("wing_right")];
        components[0].actual_size = Some(Vec3::ONE);
        components[0].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "body".into(),
            parent_anchor: "wing_mount_left".into(),
            child_anchor: "body_mount".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });
        components[1].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "body".into(),
            parent_anchor: "wing_mount_right".into(),
            child_anchor: "body_mount".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });

        let wing_left_id = component_object_id("wing_left");
        let wing_right_id = component_object_id("wing_right");

        let source_mount = anchor_named(
            "body_mount",
            Vec3::new(0.55, 0.0, 0.2),
            Vec3::NEG_X,
            Vec3::Y,
        );
        let target_mount = anchor_named("body_mount", Vec3::new(-0.55, 0.0, 0.2), Vec3::X, Vec3::Y);

        let visual = crate::object::registry::PrimitiveVisualDef::Primitive {
            mesh: crate::object::registry::MeshKey::UnitCube,
            params: None,
            color: Color::WHITE,
            unlit: false,
        };
        // Place an asymmetric feature at mount-local (right=+1, forward=+1). Under a 180-degree
        // rotation it would land at -Z, while a mirrored copy lands at +Z.
        let source_feature_pos = source_mount.transform.translation + Vec3::Z - Vec3::X;

        let mut draft = Gen3dDraft::default();
        draft.defs = vec![
            ObjectDef {
                anchors: vec![source_mount],
                parts: vec![ObjectPartDef::primitive(
                    visual.clone(),
                    Transform::from_translation(source_feature_pos),
                )],
                ..stub_def(wing_left_id, "wing_left")
            },
            ObjectDef {
                anchors: vec![target_mount],
                parts: Vec::new(),
                ..stub_def(wing_right_id, "wing_right")
            },
        ];

        copy_component_into(
            &mut components,
            &mut draft,
            0,
            1,
            Gen3dCopyMode::Detached,
            Gen3dCopyAnchorsMode::PreserveInterfaceAnchors,
            Gen3dCopyAlignmentMode::MirrorMountX,
            Transform::IDENTITY,
            None,
        )
        .expect("copy ok");

        let wing_right_def = draft
            .defs
            .iter()
            .find(|d| d.object_id == wing_right_id)
            .unwrap();
        let parts = strip_attach_refs(&wing_right_def.parts);
        assert_eq!(parts.len(), 1);
        let translated = parts[0].transform.translation;

        let expected = Vec3::new(0.45, 0.0, 1.2);
        let dp = (translated - expected).length();
        assert!(
            dp < 1e-3,
            "expected mirrored layout to keep +Z feature; got {translated:?} (dp={dp})"
        );
    }

    #[test]
    fn subtree_copy_can_expand_missing_descendants_when_partially_populated() {
        // Source subtree: leg_0_root -> leg_0_upper -> leg_0_foot (generated).
        // Target subtree: leg_1_root -> leg_1_upper exists but leg_1_upper is missing its foot child.
        // Subtree copy should clone the missing foot branch under leg_1_upper and then copy geometry.
        let mut components = vec![
            stub_component("body"),
            stub_component("leg_0_root"),
            stub_component("leg_0_upper"),
            stub_component("leg_0_foot"),
            stub_component("leg_1_root"),
            stub_component("leg_1_upper"),
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
            anchor_named_forward("to_foot", Vec3::Z),
        ];
        components[3].anchors = vec![anchor_named_forward("to_upper", Vec3::Z)];
        components[4].anchors = vec![
            anchor_named_forward("to_body", Vec3::X),
            anchor_named_forward("to_upper", Vec3::Z),
        ];
        components[5].anchors = vec![
            anchor_named_forward("to_root", Vec3::Z),
            anchor_named_forward("to_foot", Vec3::Z),
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
            parent_anchor: "to_foot".into(),
            child_anchor: "to_upper".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });
        components[4].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "body".into(),
            parent_anchor: "leg_1_mount".into(),
            child_anchor: "to_body".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });
        components[5].attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: "leg_1_root".into(),
            parent_anchor: "to_upper".into(),
            child_anchor: "to_root".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });

        // Mark source chain as generated.
        components[1].actual_size = Some(Vec3::ONE);
        components[2].actual_size = Some(Vec3::ONE);
        components[3].actual_size = Some(Vec3::ONE);

        let body_id = component_object_id("body");
        let leg0_root_id = component_object_id("leg_0_root");
        let leg0_upper_id = component_object_id("leg_0_upper");
        let leg0_foot_id = component_object_id("leg_0_foot");
        let leg1_root_id = component_object_id("leg_1_root");
        let leg1_upper_id = component_object_id("leg_1_upper");

        fn empty_def(mut def: ObjectDef) -> ObjectDef {
            def.parts.clear();
            def
        }

        let mut draft = Gen3dDraft::default();
        draft.defs =
            vec![
                {
                    let mut def = empty_def(stub_def(body_id, "body"));
                    def.anchors = vec![
                        anchor_named_forward("leg_0_mount", Vec3::Z),
                        anchor_named_forward("leg_1_mount", Vec3::Z),
                    ];
                    def.parts = vec![
                        ObjectPartDef::object_ref(leg0_root_id, Transform::IDENTITY)
                            .with_attachment(crate::object::registry::AttachmentDef {
                                parent_anchor: "leg_0_mount".into(),
                                child_anchor: "to_body".into(),
                            }),
                        ObjectPartDef::object_ref(leg1_root_id, Transform::IDENTITY)
                            .with_attachment(crate::object::registry::AttachmentDef {
                                parent_anchor: "leg_1_mount".into(),
                                child_anchor: "to_body".into(),
                            }),
                    ];
                    def
                },
                {
                    let mut def = stub_def(leg0_root_id, "leg_0_root");
                    def.anchors = vec![
                        anchor_named_forward("to_body", Vec3::Z),
                        anchor_named_forward("to_upper", Vec3::Z),
                    ];
                    def.parts.push(
                        ObjectPartDef::object_ref(leg0_upper_id, Transform::IDENTITY)
                            .with_attachment(crate::object::registry::AttachmentDef {
                                parent_anchor: "to_upper".into(),
                                child_anchor: "to_root".into(),
                            }),
                    );
                    def
                },
                {
                    let mut def = stub_def(leg0_upper_id, "leg_0_upper");
                    def.anchors = vec![
                        anchor_named_forward("to_root", Vec3::Z),
                        anchor_named_forward("to_foot", Vec3::Z),
                    ];
                    def.parts.push(
                        ObjectPartDef::object_ref(leg0_foot_id, Transform::IDENTITY)
                            .with_attachment(crate::object::registry::AttachmentDef {
                                parent_anchor: "to_foot".into(),
                                child_anchor: "to_upper".into(),
                            }),
                    );
                    def
                },
                {
                    let mut def = stub_def(leg0_foot_id, "leg_0_foot");
                    def.anchors = vec![anchor_named_forward("to_upper", Vec3::Z)];
                    def
                },
                {
                    let mut def = empty_def(stub_def(leg1_root_id, "leg_1_root"));
                    def.anchors = vec![
                        anchor_named_forward("to_body", Vec3::X),
                        anchor_named_forward("to_upper", Vec3::Z),
                    ];
                    def.parts = vec![
                        ObjectPartDef::object_ref(leg1_upper_id, Transform::IDENTITY)
                            .with_attachment(crate::object::registry::AttachmentDef {
                                parent_anchor: "to_upper".into(),
                                child_anchor: "to_root".into(),
                            }),
                    ];
                    def
                },
                {
                    let mut def = empty_def(stub_def(leg1_upper_id, "leg_1_upper"));
                    def.anchors = vec![
                        anchor_named_forward("to_root", Vec3::Z),
                        anchor_named_forward("to_foot", Vec3::Z),
                    ];
                    // No attachment refs under leg_1_upper initially (partial subtree).
                    def
                },
            ];

        copy_component_subtree_into(
            &mut components,
            &mut draft,
            1,
            4,
            Gen3dCopyMode::Detached,
            Gen3dCopyAnchorsMode::PreserveTargetAnchors,
            Gen3dCopyAlignmentMode::Rotation,
            Transform::IDENTITY,
            Gen3dSubtreeCopyMissingBranchPolicy::CloneAllMissing,
        )
        .expect("subtree copy ok");

        // leg_1_upper should now have an attachment child part.
        let leg1_upper_def_after = draft
            .defs
            .iter()
            .find(|d| d.object_id == leg1_upper_id)
            .unwrap();
        assert!(
            leg1_upper_def_after
                .parts
                .iter()
                .any(|p| matches!(p.kind, ObjectPartKind::ObjectRef { .. })
                    && p.attachment.is_some()),
            "expected expanded subtree to attach a foot under leg_1_upper"
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
            "expected subtree copy to create + populate new descendant component defs under leg_1_upper"
        );
    }
}
