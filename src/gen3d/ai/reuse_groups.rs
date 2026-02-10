use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use super::copy_component::{Gen3dCopyAnchorsMode, Gen3dCopyMode};
use super::schema::{AiReuseAnchorsJson, AiReuseGroupJson, AiReuseGroupKindJson, AiReuseModeJson};
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
    match anchors.unwrap_or(AiReuseAnchorsJson::PreserveTarget) {
        AiReuseAnchorsJson::PreserveTarget => Gen3dCopyAnchorsMode::PreserveTargetAnchors,
        AiReuseAnchorsJson::CopySource => Gen3dCopyAnchorsMode::CopySourceAnchors,
        AiReuseAnchorsJson::Unknown => Gen3dCopyAnchorsMode::PreserveTargetAnchors,
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
                    for idx in collect_subtree_indices(&children, target_root) {
                        if idx < skip_targets.len() {
                            skip_targets[idx] = true;
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
                        Transform::IDENTITY,
                    ) {
                        Ok(outcome) => {
                            report.component_copies_applied += 1;
                            report.outcomes.push(outcome);
                        }
                        Err(err) => report.errors.push(err),
                    }
                }
            }
            Gen3dReuseGroupKind::Subtree => {
                for &target_root_idx in &group.target_root_indices {
                    let subtree_indices = collect_subtree_indices(&children, target_root_idx);
                    let mut any_generated = false;
                    for idx in &subtree_indices {
                        if planned_components
                            .get(*idx)
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
                        Transform::IDENTITY,
                    ) {
                        Ok(outcomes) => {
                            report.subtree_copies_applied += 1;
                            report.component_copies_applied += outcomes.len();
                            report.outcomes.extend(outcomes);
                        }
                        Err(err) => report.errors.push(err),
                    }
                }
            }
        }
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
        child.attach_to = Some(super::super::Gen3dPlannedAttachment {
            parent: parent.to_string(),
            parent_anchor: "origin".to_string(),
            child_anchor: "origin".to_string(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });
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
}
