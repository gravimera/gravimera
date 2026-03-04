use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use super::copy_component::{Gen3dCopyAlignmentMode, Gen3dCopyAnchorsMode, Gen3dCopyMode};
use super::schema::{
    AiReuseAlignmentJson, AiReuseAnchorsJson, AiReuseGroupJson, AiReuseGroupKindJson,
    AiReuseModeJson,
};
use super::Gen3dPlannedComponent;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Gen3dReuseGroupKind {
    Component,
    Subtree,
}

#[derive(Clone, Debug)]
pub(super) struct Gen3dValidatedReuseGroup {
    pub(super) kind: Gen3dReuseGroupKind,
    pub(super) source_root_idx: usize,
    pub(super) target_root_indices: Vec<usize>,
    pub(super) alignment: Gen3dCopyAlignmentMode,
    pub(super) mode: Gen3dCopyMode,
    pub(super) anchors_mode: Gen3dCopyAnchorsMode,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Gen3dAutoCopyReport {
    pub(super) enabled: bool,
    pub(super) component_copies_applied: usize,
    pub(super) subtree_copies_applied: usize,
    pub(super) targets_skipped_already_generated: usize,
    pub(super) subtrees_skipped_partially_generated: usize,
    pub(super) preflight_mismatches: Vec<String>,
    pub(super) fallback_component_indices: Vec<usize>,
    pub(super) errors: Vec<String>,
    pub(super) outcomes: Vec<super::copy_component::Gen3dCopyComponentOutcome>,
}

fn build_name_to_index(components: &[Gen3dPlannedComponent]) -> HashMap<&str, usize> {
    let mut out: HashMap<&str, usize> = HashMap::new();
    for (idx, comp) in components.iter().enumerate() {
        out.insert(comp.name.as_str(), idx);
    }
    out
}

fn build_children_map(components: &[Gen3dPlannedComponent]) -> Vec<Vec<usize>> {
    let name_to_idx = build_name_to_index(components);
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); components.len()];
    for (idx, comp) in components.iter().enumerate() {
        let Some(att) = comp.attach_to.as_ref() else {
            continue;
        };
        let Some(&parent_idx) = name_to_idx.get(att.parent.as_str()) else {
            continue;
        };
        if parent_idx == idx {
            continue;
        }
        children[parent_idx].push(idx);
    }
    children
}

fn collect_subtree_indices(children: &[Vec<usize>], root_idx: usize) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    if root_idx >= children.len() {
        return out;
    }
    let mut stack = vec![root_idx];
    while let Some(idx) = stack.pop() {
        out.push(idx);
        for &child in children[idx].iter() {
            stack.push(child);
        }
    }
    out
}

fn is_component_leaf(components: &[Gen3dPlannedComponent], idx: usize) -> bool {
    let Some(name) = components.get(idx).map(|c| c.name.as_str()) else {
        return false;
    };
    !components.iter().any(|c| {
        c.attach_to
            .as_ref()
            .is_some_and(|att| att.parent.as_str() == name)
    })
}

fn parse_kind(kind: AiReuseGroupKindJson) -> Option<Gen3dReuseGroupKind> {
    match kind {
        AiReuseGroupKindJson::Component | AiReuseGroupKindJson::CopyComponent => {
            Some(Gen3dReuseGroupKind::Component)
        }
        AiReuseGroupKindJson::Subtree | AiReuseGroupKindJson::CopyComponentSubtree => {
            Some(Gen3dReuseGroupKind::Subtree)
        }
        AiReuseGroupKindJson::Unknown => None,
    }
}

fn parse_mode(mode: Option<AiReuseModeJson>) -> Gen3dCopyMode {
    match mode.unwrap_or(AiReuseModeJson::Detached) {
        AiReuseModeJson::Detached => Gen3dCopyMode::Detached,
        AiReuseModeJson::Linked => Gen3dCopyMode::Linked,
        AiReuseModeJson::Unknown => Gen3dCopyMode::Detached,
    }
}

fn parse_anchors_mode(anchors: Option<AiReuseAnchorsJson>) -> Gen3dCopyAnchorsMode {
    match anchors.unwrap_or(AiReuseAnchorsJson::PreserveInterfaces) {
        AiReuseAnchorsJson::PreserveTarget => Gen3dCopyAnchorsMode::PreserveTargetAnchors,
        AiReuseAnchorsJson::PreserveInterfaces => Gen3dCopyAnchorsMode::PreserveInterfaceAnchors,
        AiReuseAnchorsJson::CopySource => Gen3dCopyAnchorsMode::CopySourceAnchors,
        AiReuseAnchorsJson::Unknown => Gen3dCopyAnchorsMode::PreserveInterfaceAnchors,
    }
}

fn parse_alignment(alignment: AiReuseAlignmentJson) -> Gen3dCopyAlignmentMode {
    match alignment {
        AiReuseAlignmentJson::Rotation => Gen3dCopyAlignmentMode::Rotation,
        AiReuseAlignmentJson::MirrorMountX => Gen3dCopyAlignmentMode::MirrorMountX,
    }
}

pub(super) fn validate_reuse_groups(
    plan_groups: &[AiReuseGroupJson],
    planned_components: &[Gen3dPlannedComponent],
) -> (Vec<Gen3dValidatedReuseGroup>, Vec<String>) {
    let name_to_idx = build_name_to_index(planned_components);

    let mut out: Vec<Gen3dValidatedReuseGroup> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for (group_idx, group) in plan_groups.iter().enumerate() {
        let Some(kind) = parse_kind(group.kind) else {
            if group.kind != AiReuseGroupKindJson::Unknown {
                warnings.push(format!(
                    "reuse_groups[{group_idx}]: unsupported kind={:?}; ignoring group",
                    group.kind
                ));
            }
            continue;
        };

        let alignment = parse_alignment(group.alignment);

        let source_name = group.source.trim();
        if source_name.is_empty() {
            warnings.push(format!(
                "reuse_groups[{group_idx}]: missing source; ignoring group"
            ));
            continue;
        }
        let Some(&source_root_idx) = name_to_idx.get(source_name) else {
            warnings.push(format!(
                "reuse_groups[{group_idx}]: unknown source `{source_name}`; ignoring group"
            ));
            continue;
        };

        let mut target_root_indices: Vec<usize> = Vec::new();
        let mut seen_targets: HashSet<usize> = HashSet::new();
        for raw in group.targets.iter() {
            let name = raw.trim();
            if name.is_empty() {
                continue;
            }
            if name == source_name {
                warnings.push(format!(
                    "reuse_groups[{group_idx}]: target `{name}` equals source; skipping target"
                ));
                continue;
            }
            let Some(&idx) = name_to_idx.get(name) else {
                warnings.push(format!(
                    "reuse_groups[{group_idx}]: unknown target `{name}`; skipping target"
                ));
                continue;
            };
            if seen_targets.insert(idx) {
                target_root_indices.push(idx);
            }
        }
        if target_root_indices.is_empty() {
            warnings.push(format!(
                "reuse_groups[{group_idx}]: no valid targets for source `{source_name}`; ignoring group"
            ));
            continue;
        }

        target_root_indices
            .sort_by(|&a, &b| planned_components[a].name.cmp(&planned_components[b].name));

        let mut mode = parse_mode(group.mode);
        if kind == Gen3dReuseGroupKind::Subtree && mode == Gen3dCopyMode::Linked {
            warnings.push(format!(
                "reuse_groups[{group_idx}]: mode=linked is not supported for kind=subtree; using detached"
            ));
            mode = Gen3dCopyMode::Detached;
        }

        let anchors_mode = parse_anchors_mode(group.anchors);
        out.push(Gen3dValidatedReuseGroup {
            kind,
            source_root_idx,
            target_root_indices,
            alignment,
            mode,
            anchors_mode,
        });
    }

    (out, warnings)
}

pub(super) fn missing_only_generation_indices(
    planned_components: &[Gen3dPlannedComponent],
    reuse_groups: &[Gen3dValidatedReuseGroup],
) -> Vec<usize> {
    if planned_components.is_empty() {
        return Vec::new();
    }
    if reuse_groups.is_empty() {
        return planned_components
            .iter()
            .enumerate()
            .filter_map(|(idx, c)| c.actual_size.is_none().then_some(idx))
            .collect();
    }

    let children = build_children_map(planned_components);
    let mut required_sources = vec![false; planned_components.len()];
    let mut skip_targets = vec![false; planned_components.len()];

    for group in reuse_groups {
        let source_idx = group.source_root_idx;
        match group.kind {
            Gen3dReuseGroupKind::Component => {
                if source_idx < required_sources.len() {
                    required_sources[source_idx] = true;
                }
                let can_copy = group.mode != Gen3dCopyMode::Linked
                    || is_component_leaf(planned_components, source_idx);
                if !can_copy {
                    continue;
                }
                for &target_idx in &group.target_root_indices {
                    if target_idx < skip_targets.len() {
                        skip_targets[target_idx] = true;
                    }
                }
            }
            Gen3dReuseGroupKind::Subtree => {
                for idx in collect_subtree_indices(&children, source_idx) {
                    if idx < required_sources.len() {
                        required_sources[idx] = true;
                    }
                }
                for &target_root in &group.target_root_indices {
                    let pairs = super::copy_component::preflight_subtree_copy_pairs(
                        planned_components,
                        source_idx,
                        target_root,
                        group.anchors_mode,
                    );
                    let Ok(pairs) = pairs else {
                        continue;
                    };
                    for (_source_idx, target_idx) in pairs {
                        if target_idx < skip_targets.len() {
                            skip_targets[target_idx] = true;
                        }
                    }
                }
            }
        }
    }

    let mut out: Vec<usize> = Vec::new();
    for (idx, comp) in planned_components.iter().enumerate() {
        if comp.actual_size.is_some() {
            continue;
        }
        if skip_targets[idx] && !required_sources[idx] {
            continue;
        }
        out.push(idx);
    }

    out
}

pub(super) fn copyable_target_count(
    planned_components: &[Gen3dPlannedComponent],
    reuse_groups: &[Gen3dValidatedReuseGroup],
) -> usize {
    if planned_components.is_empty() || reuse_groups.is_empty() {
        return 0;
    }

    let children = build_children_map(planned_components);
    let mut required_sources = vec![false; planned_components.len()];
    let mut skip_targets = vec![false; planned_components.len()];

    for group in reuse_groups {
        let source_idx = group.source_root_idx;
        match group.kind {
            Gen3dReuseGroupKind::Component => {
                if source_idx < required_sources.len() {
                    required_sources[source_idx] = true;
                }
                let can_copy = group.mode != Gen3dCopyMode::Linked
                    || is_component_leaf(planned_components, source_idx);
                if !can_copy {
                    continue;
                }
                for &target_idx in &group.target_root_indices {
                    if target_idx < skip_targets.len() {
                        skip_targets[target_idx] = true;
                    }
                }
            }
            Gen3dReuseGroupKind::Subtree => {
                for idx in collect_subtree_indices(&children, source_idx) {
                    if idx < required_sources.len() {
                        required_sources[idx] = true;
                    }
                }
                for &target_root in &group.target_root_indices {
                    let pairs = super::copy_component::preflight_subtree_copy_pairs(
                        planned_components,
                        source_idx,
                        target_root,
                        group.anchors_mode,
                    );
                    let Ok(pairs) = pairs else {
                        continue;
                    };
                    for (_source_idx, target_idx) in pairs {
                        if target_idx < skip_targets.len() {
                            skip_targets[target_idx] = true;
                        }
                    }
                }
            }
        }
    }

    skip_targets
        .into_iter()
        .zip(required_sources)
        .filter(|(skip, required)| *skip && !*required)
        .count()
}

pub(super) fn apply_auto_copy(
    planned_components: &mut Vec<Gen3dPlannedComponent>,
    draft: &mut super::super::state::Gen3dDraft,
    reuse_groups: &[Gen3dValidatedReuseGroup],
) -> Gen3dAutoCopyReport {
    let mut report = Gen3dAutoCopyReport::default();
    report.enabled = !reuse_groups.is_empty();
    if reuse_groups.is_empty() {
        return report;
    }

    let children = build_children_map(planned_components);
    let mut fallback_component_indices: HashSet<usize> = HashSet::new();

    for group in reuse_groups {
        let source_root_idx = group.source_root_idx;
        if source_root_idx >= planned_components.len() {
            report
                .errors
                .push("auto_copy: source_root_idx out of range".into());
            continue;
        }

        let required_source_indices: Vec<usize> = match group.kind {
            Gen3dReuseGroupKind::Component => vec![source_root_idx],
            Gen3dReuseGroupKind::Subtree => collect_subtree_indices(&children, source_root_idx),
        };
        let mut missing_sources: Vec<String> = Vec::new();
        for idx in &required_source_indices {
            let Some(comp) = planned_components.get(*idx) else {
                continue;
            };
            if comp.actual_size.is_none() {
                missing_sources.push(comp.name.clone());
            }
        }
        if !missing_sources.is_empty() {
            report.errors.push(format!(
                "auto_copy: source not generated (kind={:?}, source_root=`{}` missing={:?})",
                group.kind, planned_components[source_root_idx].name, missing_sources
            ));
            continue;
        }

        match group.kind {
            Gen3dReuseGroupKind::Component => {
                if group.mode == Gen3dCopyMode::Linked
                    && !is_component_leaf(planned_components, source_root_idx)
                {
                    let source_name = planned_components
                        .get(source_root_idx)
                        .map(|c| c.name.as_str())
                        .unwrap_or("<missing>");
                    report.preflight_mismatches.push(format!(
                        "auto_copy: preflight mismatch for source `{source_name}`: mode=linked requires a leaf source; fallback to llm_generate_component_v1 for targets"
                    ));
                    for &target_idx in &group.target_root_indices {
                        if planned_components
                            .get(target_idx)
                            .is_some_and(|c| c.actual_size.is_none())
                        {
                            fallback_component_indices.insert(target_idx);
                        }
                    }
                    continue;
                }

                for &target_idx in &group.target_root_indices {
                    let Some(target) = planned_components.get(target_idx) else {
                        report
                            .errors
                            .push("auto_copy: target idx out of range".into());
                        continue;
                    };
                    if target.actual_size.is_some() {
                        report.targets_skipped_already_generated += 1;
                        continue;
                    }

                    match super::copy_component::copy_component_into(
                        planned_components.as_mut_slice(),
                        draft,
                        source_root_idx,
                        target_idx,
                        group.mode,
                        group.anchors_mode,
                        group.alignment,
                        Transform::IDENTITY,
                        None,
                    ) {
                        Ok(outcome) => {
                            report.component_copies_applied += 1;
                            report.outcomes.push(outcome);
                        }
                        Err(err) => {
                            report.errors.push(err);
                            fallback_component_indices.insert(target_idx);
                        }
                    }
                }
            }
            Gen3dReuseGroupKind::Subtree => {
                for &target_root_idx in &group.target_root_indices {
                    let pairs = match super::copy_component::preflight_subtree_copy_pairs(
                        planned_components,
                        source_root_idx,
                        target_root_idx,
                        group.anchors_mode,
                    ) {
                        Ok(pairs) => pairs,
                        Err(err) => {
                            let source_name = planned_components
                                .get(source_root_idx)
                                .map(|c| c.name.as_str())
                                .unwrap_or("<missing>");
                            let target_name = planned_components
                                .get(target_root_idx)
                                .map(|c| c.name.as_str())
                                .unwrap_or("<missing>");
                            report.preflight_mismatches.push(format!(
                                "auto_copy: preflight mismatch for subtree `{source_name}` -> `{target_name}`: {err}; fallback to llm_generate_component_v1"
                            ));
                            for idx in collect_subtree_indices(&children, target_root_idx) {
                                if planned_components
                                    .get(idx)
                                    .is_some_and(|c| c.actual_size.is_none())
                                {
                                    fallback_component_indices.insert(idx);
                                }
                            }
                            continue;
                        }
                    };

                    let mut any_generated = false;
                    for (_source_idx, target_idx) in pairs.iter().copied() {
                        if planned_components
                            .get(target_idx)
                            .is_some_and(|c| c.actual_size.is_some())
                        {
                            any_generated = true;
                            break;
                        }
                    }
                    if any_generated {
                        report.subtrees_skipped_partially_generated += 1;
                        continue;
                    }

                    match super::copy_component::copy_component_subtree_into(
                        planned_components,
                        draft,
                        source_root_idx,
                        target_root_idx,
                        group.mode,
                        group.anchors_mode,
                        group.alignment,
                        Transform::IDENTITY,
                    ) {
                        Ok(outcomes) => {
                            report.subtree_copies_applied += 1;
                            report.component_copies_applied += outcomes.len();
                            report.outcomes.extend(outcomes);
                        }
                        Err(err) => {
                            report.errors.push(err);
                            let children = build_children_map(planned_components);
                            for idx in collect_subtree_indices(&children, target_root_idx) {
                                if planned_components
                                    .get(idx)
                                    .is_some_and(|c| c.actual_size.is_none())
                                {
                                    fallback_component_indices.insert(idx);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if !fallback_component_indices.is_empty() {
        let mut indices: Vec<usize> = fallback_component_indices.into_iter().collect();
        indices.sort_unstable();
        report.fallback_component_indices = indices;
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::registry::AnchorDef;

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

    fn attach(child: &mut Gen3dPlannedComponent, parent: &str) {
        attach_with_edge(child, parent, "origin", "origin");
    }

    fn attach_with_edge(
        child: &mut Gen3dPlannedComponent,
        parent: &str,
        parent_anchor: &str,
        child_anchor: &str,
    ) {
        child.attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: parent.to_string(),
            parent_anchor: parent_anchor.to_string(),
            child_anchor: child_anchor.to_string(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });
    }

    fn anchor(name: &str) -> AnchorDef {
        AnchorDef {
            name: name.to_string().into(),
            transform: Transform::IDENTITY,
        }
    }

    #[test]
    fn missing_only_schedule_skips_reuse_targets() {
        let mut components = vec![
            stub_component("body"),
            stub_component("leg_0"),
            stub_component("leg_1"),
            stub_component("leg_2"),
        ];
        attach(&mut components[1], "body");
        attach(&mut components[2], "body");
        attach(&mut components[3], "body");

        let (validated, warnings) = validate_reuse_groups(
            &[AiReuseGroupJson {
                kind: AiReuseGroupKindJson::Component,
                source: "leg_0".into(),
                targets: vec!["leg_1".into(), "leg_2".into()],
                alignment: AiReuseAlignmentJson::Rotation,
                mode: None,
                anchors: None,
            }],
            &components,
        );
        assert!(warnings.is_empty());

        let indices = missing_only_generation_indices(&components, &validated);
        assert_eq!(
            indices,
            vec![0, 1],
            "expected to generate only body + source leg"
        );
    }

    #[test]
    fn subtree_reuse_skips_target_subtrees() {
        let mut components = vec![
            stub_component("body"),
            stub_component("leg0_root"),
            stub_component("leg0_upper"),
            stub_component("leg1_root"),
            stub_component("leg1_upper"),
        ];
        attach(&mut components[1], "body");
        attach(&mut components[2], "leg0_root");
        attach(&mut components[3], "body");
        attach(&mut components[4], "leg1_root");

        let (validated, warnings) = validate_reuse_groups(
            &[AiReuseGroupJson {
                kind: AiReuseGroupKindJson::Subtree,
                source: "leg0_root".into(),
                targets: vec!["leg1_root".into()],
                alignment: AiReuseAlignmentJson::Rotation,
                mode: None,
                anchors: None,
            }],
            &components,
        );
        assert!(warnings.is_empty());

        let indices = missing_only_generation_indices(&components, &validated);
        assert_eq!(
            indices,
            vec![0, 1, 2],
            "expected to generate body + full source subtree"
        );
    }

    #[test]
    fn subtree_reuse_fallback_generates_targets_on_shape_mismatch() {
        let mut components = vec![
            stub_component("body"),
            stub_component("leg0_root"),
            stub_component("leg0_foot"),
            stub_component("leg1_root"),
            stub_component("leg1_foot"),
            stub_component("leg1_spur"),
        ];
        attach_with_edge(&mut components[1], "body", "mount0", "hip");
        attach_with_edge(&mut components[2], "leg0_root", "knee", "hip");
        attach_with_edge(&mut components[3], "body", "mount1", "hip");
        attach_with_edge(&mut components[4], "leg1_root", "knee", "hip");
        // Extra branch on target root makes subtree shape incompatible with source.
        attach_with_edge(&mut components[5], "leg1_root", "spur", "hip");

        let (validated, warnings) = validate_reuse_groups(
            &[AiReuseGroupJson {
                kind: AiReuseGroupKindJson::Subtree,
                source: "leg0_root".into(),
                targets: vec!["leg1_root".into()],
                alignment: AiReuseAlignmentJson::Rotation,
                mode: None,
                anchors: None,
            }],
            &components,
        );
        assert!(warnings.is_empty());

        let indices = missing_only_generation_indices(&components, &validated);
        assert_eq!(
            indices,
            vec![0, 1, 2, 5],
            "expected to generate the extra target-only branch (spur), but skip copyable targets"
        );
    }

    #[test]
    fn subtree_reuse_still_skips_when_missing_branch_is_cloneable() {
        let mut components = vec![
            stub_component("body"),
            stub_component("leg0_root"),
            stub_component("leg0_foot"),
            stub_component("leg1_root"),
        ];
        components[3].anchors.push(anchor("knee"));
        attach_with_edge(&mut components[1], "body", "mount0", "hip");
        attach_with_edge(&mut components[2], "leg0_root", "knee", "hip");
        attach_with_edge(&mut components[3], "body", "mount1", "hip");

        let (validated, warnings) = validate_reuse_groups(
            &[AiReuseGroupJson {
                kind: AiReuseGroupKindJson::Subtree,
                source: "leg0_root".into(),
                targets: vec!["leg1_root".into()],
                alignment: AiReuseAlignmentJson::Rotation,
                mode: None,
                anchors: None,
            }],
            &components,
        );
        assert!(warnings.is_empty());

        let indices = missing_only_generation_indices(&components, &validated);
        assert_eq!(
            indices,
            vec![0, 1, 2],
            "missing target branch should stay copyable (cloneable), so target subtree remains skipped"
        );
    }

    #[test]
    fn auto_copy_subtree_reuse_allows_extra_target_branches() {
        let mut components = vec![
            stub_component("body"),
            stub_component("leg0_root"),
            stub_component("leg0_foot"),
            stub_component("leg1_root"),
            stub_component("leg1_foot"),
            stub_component("leg1_spur"),
        ];
        components[0].anchors.push(anchor("mount0"));
        components[0].anchors.push(anchor("mount1"));
        components[1].anchors.push(anchor("hip"));
        components[1].anchors.push(anchor("knee"));
        components[2].anchors.push(anchor("hip"));
        components[3].anchors.push(anchor("hip"));
        components[3].anchors.push(anchor("knee"));
        components[3].anchors.push(anchor("spur"));
        components[4].anchors.push(anchor("hip"));
        components[5].anchors.push(anchor("hip"));
        attach_with_edge(&mut components[1], "body", "mount0", "hip");
        attach_with_edge(&mut components[2], "leg0_root", "knee", "hip");
        attach_with_edge(&mut components[3], "body", "mount1", "hip");
        attach_with_edge(&mut components[4], "leg1_root", "knee", "hip");
        attach_with_edge(&mut components[5], "leg1_root", "spur", "hip");

        // Source subtree is generated; target subtree is missing.
        components[1].actual_size = Some(Vec3::ONE);
        components[2].actual_size = Some(Vec3::ONE);
        // Extra target-only branch can still be generated without blocking subtree reuse.
        components[5].actual_size = Some(Vec3::ONE);

        let (validated, warnings) = validate_reuse_groups(
            &[AiReuseGroupJson {
                kind: AiReuseGroupKindJson::Subtree,
                source: "leg0_root".into(),
                targets: vec!["leg1_root".into()],
                alignment: AiReuseAlignmentJson::Rotation,
                mode: None,
                anchors: None,
            }],
            &components,
        );
        assert!(warnings.is_empty());

        let mut draft = crate::gen3d::state::Gen3dDraft::default();
        let mut defs = Vec::new();
        for comp in components.iter() {
            let object_id = crate::object::registry::builtin_object_id(&format!(
                "gravimera/gen3d/component/{}",
                comp.name
            ));
            defs.push(crate::object::registry::ObjectDef {
                object_id,
                label: comp.name.clone().into(),
                size: Vec3::ONE,
                ground_origin_y: None,
                collider: crate::object::registry::ColliderProfile::None,
                interaction: crate::object::registry::ObjectInteraction::none(),
                aim: None,
                mobility: None,
                anchors: comp.anchors.clone(),
                parts: vec![crate::object::registry::ObjectPartDef::primitive(
                    crate::object::registry::PrimitiveVisualDef::Primitive {
                        mesh: crate::object::registry::MeshKey::UnitCube,
                        params: None,
                        color: Color::WHITE,
                        unlit: false,
                    },
                    Transform::from_scale(Vec3::ONE),
                )],
                minimap_color: None,
                health_bar_offset_y: None,
                enemy: None,
                muzzle: None,
                projectile: None,
                attack: None,
            });
        }

        // Mirror Gen3D's plan-to-draft attachment encoding: child ObjectRefs live under the parent def.
        for comp in components.iter() {
            let Some(att) = comp.attach_to.as_ref() else {
                continue;
            };
            let child_id = crate::object::registry::builtin_object_id(&format!(
                "gravimera/gen3d/component/{}",
                comp.name
            ));
            let parent_id = crate::object::registry::builtin_object_id(&format!(
                "gravimera/gen3d/component/{}",
                att.parent
            ));
            let part = crate::object::registry::ObjectPartDef::object_ref(child_id, att.offset)
                .with_attachment(crate::object::registry::AttachmentDef {
                    parent_anchor: att.parent_anchor.clone().into(),
                    child_anchor: att.child_anchor.clone().into(),
                });
            if let Some(parent_def) = defs.iter_mut().find(|d| d.object_id == parent_id) {
                parent_def.parts.push(part);
            }
        }
        draft.defs = defs;

        let report = apply_auto_copy(&mut components, &mut draft, &validated);
        assert!(
            report.errors.is_empty(),
            "expected subtree reuse to succeed, got errors: {:?}",
            report.errors
        );
        assert!(report.preflight_mismatches.is_empty());
        assert!(report.fallback_component_indices.is_empty());
        assert_eq!(report.subtree_copies_applied, 1);
        assert_eq!(report.component_copies_applied, 2);

        assert!(
            components[3].actual_size.is_some(),
            "expected subtree reuse to copy leg1_root"
        );
        assert!(
            components[4].actual_size.is_some(),
            "expected subtree reuse to copy leg1_foot"
        );
        assert!(
            components[5].actual_size.is_some(),
            "expected pre-generated extra branch to remain generated"
        );

        let leg1_root_id =
            crate::object::registry::builtin_object_id("gravimera/gen3d/component/leg1_root");
        let leg1_spur_id =
            crate::object::registry::builtin_object_id("gravimera/gen3d/component/leg1_spur");
        let leg1_root_def = draft
            .defs
            .iter()
            .find(|d| d.object_id == leg1_root_id)
            .expect("leg1_root def");
        assert!(
            leg1_root_def.parts.iter().any(|p| {
                matches!(p.kind, crate::object::registry::ObjectPartKind::ObjectRef { object_id } if object_id == leg1_spur_id)
                    && p.attachment.as_ref().is_some_and(|att| {
                        att.parent_anchor.as_ref() == "spur" && att.child_anchor.as_ref() == "hip"
                    })
            }),
            "expected subtree reuse to preserve extra spur attachment ref"
        );
    }
}
