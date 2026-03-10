use bevy::prelude::*;

use super::{Gen3dPlannedAttachment, Gen3dPlannedComponent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PreserveEditPolicy {
    Additive,
    AllowOffsets,
    AllowRewire,
}

impl PreserveEditPolicy {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Additive => "additive",
            Self::AllowOffsets => "allow_offsets",
            Self::AllowRewire => "allow_rewire",
        }
    }
}

pub(super) fn parse_preserve_edit_policy(raw: Option<&str>) -> Option<PreserveEditPolicy> {
    match raw.unwrap_or("additive").trim() {
        "additive" => Some(PreserveEditPolicy::Additive),
        "allow_offsets" => Some(PreserveEditPolicy::AllowOffsets),
        "allow_rewire" => Some(PreserveEditPolicy::AllowRewire),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum PreservePlanViolationKind {
    MissingAllowList,
    UnknownAllowListComponent,
    AttachToPresenceChanged,
    RewireInterfaceChanged,
    OffsetChanged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PreservePlanViolation {
    pub(super) kind: PreservePlanViolationKind,
    pub(super) component: String,
    pub(super) field: String,
    pub(super) old: String,
    pub(super) new: String,
}

fn approx_eq_f32(a: f32, b: f32, eps: f32) -> bool {
    (a - b).abs() <= eps
}

fn approx_eq_vec3(a: Vec3, b: Vec3, eps: f32) -> bool {
    approx_eq_f32(a.x, b.x, eps) && approx_eq_f32(a.y, b.y, eps) && approx_eq_f32(a.z, b.z, eps)
}

fn approx_eq_quat(a: Quat, b: Quat, eps: f32) -> bool {
    // Quats are equivalent up to sign. Compare both directions deterministically.
    let a = a.normalize();
    let b = b.normalize();
    let d1 = (a.x - b.x).abs() + (a.y - b.y).abs() + (a.z - b.z).abs() + (a.w - b.w).abs();
    let d2 = (a.x + b.x).abs() + (a.y + b.y).abs() + (a.z + b.z).abs() + (a.w + b.w).abs();
    d1.min(d2) <= eps * 4.0
}

fn transform_equal(a: &Transform, b: &Transform) -> bool {
    const EPS: f32 = 1e-6;
    approx_eq_vec3(a.translation, b.translation, EPS)
        && approx_eq_vec3(a.scale, b.scale, EPS)
        && approx_eq_quat(a.rotation, b.rotation, EPS)
}

fn attachment_interface_key(att: &Gen3dPlannedAttachment) -> (&str, &str, &str) {
    (
        att.parent.trim(),
        att.parent_anchor.trim(),
        att.child_anchor.trim(),
    )
}

pub(super) fn validate_preserve_mode_plan_diff(
    old_components: &[Gen3dPlannedComponent],
    new_components: &[Gen3dPlannedComponent],
    policy: PreserveEditPolicy,
    rewire_components: &[String],
) -> Vec<PreservePlanViolation> {
    let mut violations: Vec<PreservePlanViolation> = Vec::new();
    let old_by_name: std::collections::HashMap<&str, &Gen3dPlannedComponent> = old_components
        .iter()
        .map(|c| (c.name.as_str(), c))
        .collect();
    let new_by_name: std::collections::HashMap<&str, &Gen3dPlannedComponent> = new_components
        .iter()
        .map(|c| (c.name.as_str(), c))
        .collect();

    let mut allow_rewire: std::collections::HashSet<&str> = std::collections::HashSet::new();
    if policy == PreserveEditPolicy::AllowRewire {
        for name in rewire_components.iter() {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                continue;
            }
            allow_rewire.insert(trimmed);
        }
        if allow_rewire.is_empty() {
            violations.push(PreservePlanViolation {
                kind: PreservePlanViolationKind::MissingAllowList,
                component: "<constraints>".into(),
                field: "constraints.rewire_components".into(),
                old: "(required for allow_rewire)".into(),
                new: "(missing/empty)".into(),
            });
        }
        for name in allow_rewire.iter().copied().collect::<Vec<_>>() {
            if !old_by_name.contains_key(name) {
                violations.push(PreservePlanViolation {
                    kind: PreservePlanViolationKind::UnknownAllowListComponent,
                    component: "<constraints>".into(),
                    field: "constraints.rewire_components".into(),
                    old: "(existing component name)".into(),
                    new: name.to_string(),
                });
            }
        }
    }

    for (name, old) in old_by_name.iter() {
        let Some(new) = new_by_name.get(name) else {
            continue;
        };
        let allow_this_rewire =
            policy == PreserveEditPolicy::AllowRewire && allow_rewire.contains(*name);

        match (old.attach_to.as_ref(), new.attach_to.as_ref()) {
            (None, None) => {}
            (Some(_), None) | (None, Some(_)) => {
                violations.push(PreservePlanViolation {
                    kind: PreservePlanViolationKind::AttachToPresenceChanged,
                    component: (*name).to_string(),
                    field: "attach_to".into(),
                    old: if old.attach_to.is_some() {
                        "Some(...)".into()
                    } else {
                        "None".into()
                    },
                    new: if new.attach_to.is_some() {
                        "Some(...)".into()
                    } else {
                        "None".into()
                    },
                });
            }
            (Some(old_att), Some(new_att)) => {
                let (old_parent, old_parent_anchor, old_child_anchor) =
                    attachment_interface_key(old_att);
                let (new_parent, new_parent_anchor, new_child_anchor) =
                    attachment_interface_key(new_att);

                let interface_changed = old_parent != new_parent
                    || old_parent_anchor != new_parent_anchor
                    || old_child_anchor != new_child_anchor;
                if interface_changed && !(allow_this_rewire) {
                    violations.push(PreservePlanViolation {
                        kind: PreservePlanViolationKind::RewireInterfaceChanged,
                        component: (*name).to_string(),
                        field: "attach_to.(parent,parent_anchor,child_anchor)".into(),
                        old: format!(
                            "parent={old_parent} parent_anchor={old_parent_anchor} child_anchor={old_child_anchor}"
                        ),
                        new: format!(
                            "parent={new_parent} parent_anchor={new_parent_anchor} child_anchor={new_child_anchor}"
                        ),
                    });
                }

                let offset_changed = !transform_equal(&old_att.offset, &new_att.offset);
                let offset_allowed = match policy {
                    PreserveEditPolicy::Additive => false,
                    PreserveEditPolicy::AllowOffsets => !interface_changed,
                    PreserveEditPolicy::AllowRewire => allow_this_rewire,
                };
                if offset_changed && !offset_allowed {
                    violations.push(PreservePlanViolation {
                        kind: PreservePlanViolationKind::OffsetChanged,
                        component: (*name).to_string(),
                        field: "attach_to.offset".into(),
                        old: format!(
                            "pos={:?} rot={:?} scale={:?}",
                            old_att.offset.translation,
                            old_att.offset.rotation,
                            old_att.offset.scale
                        ),
                        new: format!(
                            "pos={:?} rot={:?} scale={:?}",
                            new_att.offset.translation,
                            new_att.offset.rotation,
                            new_att.offset.scale
                        ),
                    });
                }
            }
        }
    }

    violations.sort_by(|a, b| {
        (
            a.component.as_str(),
            &a.kind,
            a.field.as_str(),
            a.old.as_str(),
            a.new.as_str(),
        )
            .cmp(&(
                b.component.as_str(),
                &b.kind,
                b.field.as_str(),
                b.old.as_str(),
                b.new.as_str(),
            ))
    });
    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comp(name: &str, attach_to: Option<Gen3dPlannedAttachment>) -> Gen3dPlannedComponent {
        Gen3dPlannedComponent {
            display_name: name.into(),
            name: name.into(),
            purpose: "".into(),
            modeling_notes: "".into(),
            pos: Vec3::ZERO,
            rot: Quat::IDENTITY,
            planned_size: Vec3::ONE,
            actual_size: None,
            anchors: Vec::new(),
            contacts: Vec::new(),
            attach_to,
        }
    }

    fn att(
        parent: &str,
        parent_anchor: &str,
        child_anchor: &str,
        offset_z: f32,
    ) -> Gen3dPlannedAttachment {
        Gen3dPlannedAttachment {
            parent: parent.into(),
            parent_anchor: parent_anchor.into(),
            child_anchor: child_anchor.into(),
            offset: Transform::from_translation(Vec3::new(0.0, 0.0, offset_z)),
            joint: None,
            animations: Vec::new(),
        }
    }

    #[test]
    fn additive_rejects_rewire_and_offset_changes() {
        let old = vec![
            comp("torso", None),
            comp(
                "neck",
                Some(att("torso", "neck_socket", "torso_socket", -0.03)),
            ),
            comp(
                "head",
                Some(att("neck", "head_socket", "neck_socket", -0.02)),
            ),
        ];
        let new = vec![
            comp("torso", None),
            comp(
                "neck",
                Some(att("torso", "neck_mount", "torso_mount", -0.01)),
            ), // rewire + offset
            comp(
                "head",
                Some(att("neck", "head_socket", "neck_socket", -0.02)),
            ),
            comp(
                "santa_hat",
                Some(att("head", "hat_mount", "head_mount", -0.01)),
            ), // new component ok
        ];

        let v = validate_preserve_mode_plan_diff(&old, &new, PreserveEditPolicy::Additive, &[]);
        assert!(v
            .iter()
            .any(|i| i.kind == PreservePlanViolationKind::RewireInterfaceChanged));
        assert!(v
            .iter()
            .any(|i| i.kind == PreservePlanViolationKind::OffsetChanged));
    }

    #[test]
    fn allow_offsets_allows_offset_changes_but_not_rewire() {
        let old = vec![
            comp("torso", None),
            comp(
                "neck",
                Some(att("torso", "neck_socket", "torso_socket", -0.03)),
            ),
        ];
        let new = vec![
            comp("torso", None),
            comp(
                "neck",
                Some(att("torso", "neck_socket", "torso_socket", -0.01)),
            ), // offset only
        ];

        let v = validate_preserve_mode_plan_diff(&old, &new, PreserveEditPolicy::AllowOffsets, &[]);
        assert!(v.is_empty(), "{v:?}");

        let new_rewire = vec![
            comp("torso", None),
            comp(
                "neck",
                Some(att("torso", "neck_mount", "torso_socket", -0.03)),
            ),
        ];
        let v = validate_preserve_mode_plan_diff(
            &old,
            &new_rewire,
            PreserveEditPolicy::AllowOffsets,
            &[],
        );
        assert!(v
            .iter()
            .any(|i| i.kind == PreservePlanViolationKind::RewireInterfaceChanged));
    }

    #[test]
    fn allow_rewire_requires_non_empty_allowlist_and_enforces_it() {
        let old = vec![
            comp("torso", None),
            comp(
                "neck",
                Some(att("torso", "neck_socket", "torso_socket", -0.03)),
            ),
            comp(
                "head",
                Some(att("neck", "head_socket", "neck_socket", -0.02)),
            ),
        ];

        let new = vec![
            comp("torso", None),
            comp(
                "neck",
                Some(att("torso", "neck_mount", "torso_mount", -0.01)),
            ), // rewire
            comp("head", Some(att("neck", "head_mount", "neck_mount", -0.01))), // rewire too
        ];

        let v = validate_preserve_mode_plan_diff(&old, &new, PreserveEditPolicy::AllowRewire, &[]);
        assert!(v
            .iter()
            .any(|i| i.kind == PreservePlanViolationKind::MissingAllowList));

        let allow_neck = vec!["neck".to_string()];
        let v = validate_preserve_mode_plan_diff(
            &old,
            &new,
            PreserveEditPolicy::AllowRewire,
            &allow_neck,
        );
        assert!(v
            .iter()
            .any(|i| i.kind == PreservePlanViolationKind::RewireInterfaceChanged));

        let allow_both = vec!["neck".to_string(), "head".to_string()];
        let v = validate_preserve_mode_plan_diff(
            &old,
            &new,
            PreserveEditPolicy::AllowRewire,
            &allow_both,
        );
        assert!(!v
            .iter()
            .any(|i| i.kind == PreservePlanViolationKind::RewireInterfaceChanged));
    }
}
