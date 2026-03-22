use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

use crate::object::registry::{
    AnchorDef, PartAnimationDef, PartAnimationDriver, PartAnimationKeyframeDef, PartAnimationSlot,
    PartAnimationSpec,
};

use super::schema::{AiColliderJson, AiContactJson, AiJointJson, AiMotionAuthoringJsonV1};
use super::{Gen3dAgentWorkspace, Gen3dAiJob, Gen3dPlannedAttachment, Gen3dPlannedComponent};

const GEN3D_EDIT_BUNDLE_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct Gen3dEditBundleV1 {
    #[serde(default)]
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) root_prefab_id_uuid: String,
    #[serde(default)]
    pub(crate) created_at_ms: u64,
    #[serde(default)]
    pub(crate) plan_hash: String,
    #[serde(default)]
    pub(crate) assembly_rev: u32,
    #[serde(default)]
    pub(crate) assembly_notes: String,
    #[serde(default)]
    pub(crate) plan_collider: Option<AiColliderJson>,
    #[serde(default)]
    pub(crate) planned_components: Vec<Gen3dPlannedComponentBundleV1>,
    #[serde(default)]
    pub(crate) rig_move_cycle_m: Option<f32>,
    #[serde(default)]
    pub(crate) motion_authoring: Option<AiMotionAuthoringJsonV1>,
    #[serde(default)]
    pub(crate) reuse_group_warnings: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct Gen3dPlannedComponentBundleV1 {
    #[serde(default)]
    pub(crate) display_name: String,
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) purpose: String,
    #[serde(default)]
    pub(crate) modeling_notes: String,
    #[serde(default)]
    pub(crate) pos: [f32; 3],
    #[serde(default)]
    pub(crate) rot_quat_xyzw: [f32; 4],
    #[serde(default)]
    pub(crate) planned_size: [f32; 3],
    #[serde(default)]
    pub(crate) actual_size: Option<[f32; 3]>,
    #[serde(default)]
    pub(crate) anchors: Vec<AnchorDefBundleV1>,
    #[serde(default)]
    pub(crate) contacts: Vec<AiContactJson>,
    /// Animation slots applied on the implicit draft-root -> root-component object_ref edge.
    /// Only meaningful for the root component (`attach_to=None`).
    #[serde(default)]
    pub(crate) root_animations: Vec<PartAnimationSlotBundleV1>,
    #[serde(default)]
    pub(crate) attach_to: Option<Gen3dPlannedAttachmentBundleV1>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct Gen3dPlannedAttachmentBundleV1 {
    #[serde(default)]
    pub(crate) parent: String,
    #[serde(default)]
    pub(crate) parent_anchor: String,
    #[serde(default)]
    pub(crate) child_anchor: String,
    #[serde(default)]
    pub(crate) offset: TransformBundleV1,
    #[serde(default)]
    pub(crate) joint: Option<AiJointJson>,
    #[serde(default)]
    pub(crate) animations: Vec<PartAnimationSlotBundleV1>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct AnchorDefBundleV1 {
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) transform: TransformBundleV1,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub(crate) struct TransformBundleV1 {
    #[serde(default)]
    pub(crate) translation: [f32; 3],
    #[serde(default)]
    pub(crate) rotation_quat_xyzw: [f32; 4],
    #[serde(default = "default_vec3_one")]
    pub(crate) scale: [f32; 3],
}

fn default_vec3_one() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct PartAnimationSlotBundleV1 {
    #[serde(default)]
    pub(crate) channel: String,
    #[serde(default)]
    pub(crate) spec: PartAnimationSpecBundleV1,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct PartAnimationSpecBundleV1 {
    #[serde(default)]
    pub(crate) driver: PartAnimationDriverBundleV1,
    #[serde(default)]
    pub(crate) speed_scale: f32,
    #[serde(default)]
    pub(crate) time_offset_units: f32,
    #[serde(default)]
    pub(crate) clip: PartAnimationDefBundleV1,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PartAnimationDriverBundleV1 {
    #[default]
    Always,
    MovePhase,
    MoveDistance,
    AttackTime,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PartAnimationSpinAxisSpaceBundleV1 {
    #[default]
    Join,
    ChildLocal,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum PartAnimationDefBundleV1 {
    Loop {
        duration_secs: f32,
        #[serde(default)]
        keyframes: Vec<PartAnimationKeyframeDefBundleV1>,
    },
    Once {
        duration_secs: f32,
        #[serde(default)]
        keyframes: Vec<PartAnimationKeyframeDefBundleV1>,
    },
    PingPong {
        duration_secs: f32,
        #[serde(default)]
        keyframes: Vec<PartAnimationKeyframeDefBundleV1>,
    },
    Spin {
        axis: [f32; 3],
        radians_per_unit: f32,
        #[serde(default)]
        axis_space: PartAnimationSpinAxisSpaceBundleV1,
    },
}

impl Default for PartAnimationDefBundleV1 {
    fn default() -> Self {
        Self::Loop {
            duration_secs: 0.0,
            keyframes: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct PartAnimationKeyframeDefBundleV1 {
    #[serde(default)]
    pub(crate) time_secs: f32,
    #[serde(default)]
    pub(crate) delta: TransformBundleV1,
}

pub(crate) fn gen3d_build_edit_bundle_v1(
    job: &Gen3dAiJob,
    root_prefab_id: u128,
) -> Gen3dEditBundleV1 {
    let created_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0);

    Gen3dEditBundleV1 {
        version: GEN3D_EDIT_BUNDLE_FORMAT_VERSION,
        root_prefab_id_uuid: Uuid::from_u128(root_prefab_id).to_string(),
        created_at_ms,
        plan_hash: job.plan_hash.clone(),
        assembly_rev: job.assembly_rev,
        assembly_notes: job.assembly_notes.clone(),
        plan_collider: job.plan_collider.clone(),
        planned_components: job
            .planned_components
            .iter()
            .map(Gen3dPlannedComponentBundleV1::from_component)
            .collect(),
        rig_move_cycle_m: job.rig_move_cycle_m,
        motion_authoring: job.motion_authoring.clone(),
        reuse_group_warnings: job.reuse_group_warnings.clone(),
    }
}

pub(crate) fn gen3d_write_edit_bundle_v1(
    path: &Path,
    bundle: &Gen3dEditBundleV1,
) -> Result<(), String> {
    let data = serde_json::to_string_pretty(bundle).map_err(|err| err.to_string())?;
    std::fs::write(path, data)
        .map_err(|err| format!("Failed to write {}: {err}", path.display()))?;
    Ok(())
}

pub(crate) fn gen3d_load_edit_bundle_v1(path: &Path) -> Result<Gen3dEditBundleV1, String> {
    let bytes =
        std::fs::read(path).map_err(|err| format!("Failed to read {}: {err}", path.display()))?;
    let mut bundle: Gen3dEditBundleV1 =
        serde_json::from_slice(&bytes).map_err(|err| format!("Invalid JSON: {err}"))?;
    if bundle.version == 0 {
        bundle.version = GEN3D_EDIT_BUNDLE_FORMAT_VERSION;
    }
    if bundle.version != GEN3D_EDIT_BUNDLE_FORMAT_VERSION {
        return Err(format!(
            "Unsupported gen3d_edit_bundle_v1 version {} (expected {})",
            bundle.version, GEN3D_EDIT_BUNDLE_FORMAT_VERSION
        ));
    }
    Ok(bundle)
}

pub(crate) fn gen3d_hydrate_seeded_job_from_edit_bundle_v1(
    job: &mut Gen3dAiJob,
    bundle: &Gen3dEditBundleV1,
    draft_defs: &[crate::object::registry::ObjectDef],
) -> Result<(), String> {
    let planned: Vec<Gen3dPlannedComponent> = bundle
        .planned_components
        .iter()
        .map(Gen3dPlannedComponentBundleV1::to_component)
        .collect::<Result<Vec<_>, _>>()?;

    job.planned_components = planned;
    hydrate_planned_attachment_animations_from_defs(&mut job.planned_components, draft_defs);
    job.plan_hash = bundle.plan_hash.trim().to_string();
    job.assembly_rev = bundle.assembly_rev;
    job.assembly_notes = bundle.assembly_notes.clone();
    job.plan_collider = bundle.plan_collider.clone();
    job.rig_move_cycle_m = bundle.rig_move_cycle_m;
    job.motion_authoring = bundle.motion_authoring.clone();
    job.reuse_group_warnings = bundle.reuse_group_warnings.clone();

    // Ensure the agent has a workspace so prompt summaries are non-empty after restart.
    let workspace_id = "main".to_string();
    job.agent.active_workspace_id = workspace_id.clone();
    job.agent.workspaces.clear();
    job.agent.workspaces.insert(
        workspace_id.clone(),
        Gen3dAgentWorkspace {
            name: workspace_id,
            defs: draft_defs.to_vec(),
            planned_components: job.planned_components.clone(),
            plan_hash: job.plan_hash.clone(),
            assembly_rev: job.assembly_rev,
            assembly_notes: job.assembly_notes.clone(),
            plan_collider: job.plan_collider.clone(),
            rig_move_cycle_m: job.rig_move_cycle_m,
            motion_authoring: job.motion_authoring.clone(),
            reuse_groups: Vec::new(),
            reuse_group_warnings: job.reuse_group_warnings.clone(),
        },
    );

    Ok(())
}

fn hydrate_planned_attachment_animations_from_defs(
    planned_components: &mut [Gen3dPlannedComponent],
    draft_defs: &[crate::object::registry::ObjectDef],
) {
    use crate::object::registry::ObjectPartKind;

    let mut defs_by_id: std::collections::HashMap<u128, &crate::object::registry::ObjectDef> =
        std::collections::HashMap::new();
    for def in draft_defs {
        defs_by_id.insert(def.object_id, def);
    }

    // Root animation slots live on the implicit draft-root -> root-component object_ref part.
    if let Some(root_comp) = planned_components
        .iter_mut()
        .find(|c| c.attach_to.is_none())
    {
        if root_comp.root_animations.is_empty() {
            let root_def_id = super::super::gen3d_draft_object_id();
            let root_component_id = crate::object::registry::builtin_object_id(&format!(
                "gravimera/gen3d/component/{}",
                root_comp.name.trim()
            ));
            if let Some(root_def) = defs_by_id.get(&root_def_id) {
                for part in root_def.parts.iter() {
                    let ObjectPartKind::ObjectRef { object_id } = part.kind else {
                        continue;
                    };
                    if object_id != root_component_id {
                        continue;
                    }
                    if part.animations.is_empty() {
                        continue;
                    }
                    root_comp.root_animations = part.animations.clone();
                    break;
                }
            }
        }
    }

    for comp in planned_components.iter_mut() {
        let Some(att) = comp.attach_to.as_mut() else {
            continue;
        };
        if !att.animations.is_empty() {
            continue;
        }

        let parent_object_id = crate::object::registry::builtin_object_id(&format!(
            "gravimera/gen3d/component/{}",
            att.parent.trim()
        ));
        let child_object_id = crate::object::registry::builtin_object_id(&format!(
            "gravimera/gen3d/component/{}",
            comp.name.trim()
        ));

        let Some(parent_def) = defs_by_id.get(&parent_object_id) else {
            continue;
        };

        // If the draft already contains authored animation slots on the parent->child object_ref
        // edge, copy them into the planned attachment so state summaries and subsequent plan merges
        // see the existing motion (restart-safe).
        for part in parent_def.parts.iter() {
            let ObjectPartKind::ObjectRef { object_id } = part.kind else {
                continue;
            };
            if object_id != child_object_id {
                continue;
            }
            if part.animations.is_empty() {
                continue;
            }
            att.animations = part.animations.clone();
            break;
        }
    }
}

impl Gen3dPlannedComponentBundleV1 {
    fn from_component(c: &Gen3dPlannedComponent) -> Self {
        Self {
            display_name: c.display_name.clone(),
            name: c.name.clone(),
            purpose: c.purpose.clone(),
            modeling_notes: c.modeling_notes.clone(),
            pos: [c.pos.x, c.pos.y, c.pos.z],
            rot_quat_xyzw: [c.rot.x, c.rot.y, c.rot.z, c.rot.w],
            planned_size: [c.planned_size.x, c.planned_size.y, c.planned_size.z],
            actual_size: c.actual_size.map(|v| [v.x, v.y, v.z]),
            anchors: c
                .anchors
                .iter()
                .map(AnchorDefBundleV1::from_anchor)
                .collect(),
            contacts: c.contacts.clone(),
            root_animations: c
                .root_animations
                .iter()
                .map(PartAnimationSlotBundleV1::from_slot)
                .collect(),
            attach_to: c
                .attach_to
                .as_ref()
                .map(Gen3dPlannedAttachmentBundleV1::from_attachment),
        }
    }

    fn to_component(&self) -> Result<Gen3dPlannedComponent, String> {
        Ok(Gen3dPlannedComponent {
            display_name: self.display_name.clone(),
            name: self.name.clone(),
            purpose: self.purpose.clone(),
            modeling_notes: self.modeling_notes.clone(),
            pos: Vec3::new(self.pos[0], self.pos[1], self.pos[2]),
            rot: {
                let q = Quat::from_xyzw(
                    self.rot_quat_xyzw[0],
                    self.rot_quat_xyzw[1],
                    self.rot_quat_xyzw[2],
                    self.rot_quat_xyzw[3],
                );
                if q.length_squared() > 1e-6 {
                    q.normalize()
                } else {
                    Quat::IDENTITY
                }
            },
            planned_size: Vec3::new(
                self.planned_size[0],
                self.planned_size[1],
                self.planned_size[2],
            ),
            actual_size: self.actual_size.map(|v| Vec3::new(v[0], v[1], v[2])),
            anchors: self
                .anchors
                .iter()
                .map(AnchorDefBundleV1::to_anchor)
                .collect::<Result<Vec<_>, _>>()?,
            contacts: self.contacts.clone(),
            root_animations: self
                .root_animations
                .iter()
                .map(PartAnimationSlotBundleV1::to_slot)
                .collect::<Result<Vec<_>, _>>()?,
            attach_to: self
                .attach_to
                .as_ref()
                .map(Gen3dPlannedAttachmentBundleV1::to_attachment)
                .transpose()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;
    use crate::object::registry::{
        builtin_object_id, ColliderProfile, ObjectDef, ObjectInteraction, ObjectPartDef,
        PartAnimationDef, PartAnimationDriver, PartAnimationSlot, PartAnimationSpec,
    };

    #[test]
    fn hydrate_planned_attachment_animations_from_defs_copies_object_ref_part_slots() {
        let parent_name = "arm_fl";
        let child_name = "rotor_fl";
        let parent_object_id =
            builtin_object_id(&format!("gravimera/gen3d/component/{parent_name}"));
        let child_object_id = builtin_object_id(&format!("gravimera/gen3d/component/{child_name}"));

        let mut object_ref = ObjectPartDef::object_ref(child_object_id, Transform::IDENTITY);
        object_ref.animations.push(PartAnimationSlot {
            channel: Cow::Borrowed("move"),
            spec: PartAnimationSpec {
                driver: PartAnimationDriver::MovePhase,
                speed_scale: 1.0,
                time_offset_units: 0.0,
                clip: PartAnimationDef::Spin {
                    axis: Vec3::Y,
                    radians_per_unit: 1.0,
                    axis_space: crate::object::registry::PartAnimationSpinAxisSpace::Join,
                },
            },
        });

        let parent_def = ObjectDef {
            object_id: parent_object_id,
            label: Cow::Borrowed("parent"),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: Vec::new(),
            parts: vec![object_ref],
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        };
        let child_def = ObjectDef {
            object_id: child_object_id,
            label: Cow::Borrowed("child"),
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
        };

        let draft_defs = vec![parent_def, child_def];
        let mut planned = vec![Gen3dPlannedComponent {
            display_name: child_name.into(),
            name: child_name.into(),
            purpose: String::new(),
            modeling_notes: String::new(),
            pos: Vec3::ZERO,
            rot: Quat::IDENTITY,
            planned_size: Vec3::ONE,
            actual_size: Some(Vec3::ONE),
            anchors: Vec::new(),
            contacts: Vec::new(),
            root_animations: Vec::new(),
            attach_to: Some(Gen3dPlannedAttachment {
                parent: parent_name.into(),
                parent_anchor: "rotor_mount".into(),
                child_anchor: "arm_mount".into(),
                offset: Transform::IDENTITY,
                joint: None,
                animations: Vec::new(),
            }),
        }];

        hydrate_planned_attachment_animations_from_defs(&mut planned, &draft_defs);

        let animations = planned[0]
            .attach_to
            .as_ref()
            .expect("expected attach_to")
            .animations
            .as_slice();
        assert_eq!(animations.len(), 1);
        assert_eq!(animations[0].channel.as_ref(), "move");
    }
}

impl AnchorDefBundleV1 {
    fn from_anchor(a: &AnchorDef) -> Self {
        Self {
            name: a.name.to_string(),
            transform: TransformBundleV1::from_transform(a.transform),
        }
    }

    fn to_anchor(&self) -> Result<AnchorDef, String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("Anchor name must be non-empty".into());
        }
        Ok(AnchorDef {
            name: name.to_string().into(),
            transform: self.transform.to_transform(),
        })
    }
}

impl TransformBundleV1 {
    fn from_transform(t: Transform) -> Self {
        Self {
            translation: [t.translation.x, t.translation.y, t.translation.z],
            rotation_quat_xyzw: [t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w],
            scale: [t.scale.x, t.scale.y, t.scale.z],
        }
    }

    fn to_transform(&self) -> Transform {
        Transform {
            translation: Vec3::new(
                self.translation[0],
                self.translation[1],
                self.translation[2],
            ),
            rotation: {
                let q = Quat::from_xyzw(
                    self.rotation_quat_xyzw[0],
                    self.rotation_quat_xyzw[1],
                    self.rotation_quat_xyzw[2],
                    self.rotation_quat_xyzw[3],
                );
                if q.length_squared() > 1e-6 {
                    q.normalize()
                } else {
                    Quat::IDENTITY
                }
            },
            scale: Vec3::new(self.scale[0], self.scale[1], self.scale[2]),
        }
    }
}

impl Gen3dPlannedAttachmentBundleV1 {
    fn from_attachment(a: &Gen3dPlannedAttachment) -> Self {
        Self {
            parent: a.parent.clone(),
            parent_anchor: a.parent_anchor.clone(),
            child_anchor: a.child_anchor.clone(),
            offset: TransformBundleV1::from_transform(a.offset),
            joint: a.joint.clone(),
            animations: a
                .animations
                .iter()
                .map(PartAnimationSlotBundleV1::from_slot)
                .collect(),
        }
    }

    fn to_attachment(&self) -> Result<Gen3dPlannedAttachment, String> {
        let parent = self.parent.trim();
        if parent.is_empty() {
            return Err("Attachment parent must be non-empty".into());
        }
        let parent_anchor = self.parent_anchor.trim();
        let child_anchor = self.child_anchor.trim();
        if parent_anchor.is_empty() || child_anchor.is_empty() {
            return Err("Attachment anchors must be non-empty".into());
        }

        Ok(Gen3dPlannedAttachment {
            parent: parent.to_string(),
            parent_anchor: parent_anchor.to_string(),
            child_anchor: child_anchor.to_string(),
            offset: self.offset.to_transform(),
            joint: self.joint.clone(),
            animations: self
                .animations
                .iter()
                .map(PartAnimationSlotBundleV1::to_slot)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

impl PartAnimationSlotBundleV1 {
    fn from_slot(slot: &PartAnimationSlot) -> Self {
        Self {
            channel: slot.channel.to_string(),
            spec: PartAnimationSpecBundleV1::from_spec(&slot.spec),
        }
    }

    fn to_slot(&self) -> Result<PartAnimationSlot, String> {
        let channel = self.channel.trim();
        if channel.is_empty() {
            return Err("Animation channel must be non-empty".into());
        }
        Ok(PartAnimationSlot {
            channel: channel.to_string().into(),
            spec: self.spec.to_spec()?,
        })
    }
}

impl PartAnimationSpecBundleV1 {
    fn from_spec(spec: &PartAnimationSpec) -> Self {
        Self {
            driver: PartAnimationDriverBundleV1::from_driver(spec.driver),
            speed_scale: spec.speed_scale,
            time_offset_units: spec.time_offset_units,
            clip: PartAnimationDefBundleV1::from_clip(&spec.clip),
        }
    }

    fn to_spec(&self) -> Result<PartAnimationSpec, String> {
        Ok(PartAnimationSpec {
            driver: self.driver.to_driver(),
            speed_scale: self.speed_scale,
            time_offset_units: self.time_offset_units,
            clip: self.clip.to_clip()?,
        })
    }
}

impl PartAnimationDriverBundleV1 {
    fn from_driver(driver: PartAnimationDriver) -> Self {
        match driver {
            PartAnimationDriver::Always => Self::Always,
            PartAnimationDriver::MovePhase => Self::MovePhase,
            PartAnimationDriver::MoveDistance => Self::MoveDistance,
            PartAnimationDriver::AttackTime => Self::AttackTime,
        }
    }

    fn to_driver(self) -> PartAnimationDriver {
        match self {
            Self::Always => PartAnimationDriver::Always,
            Self::MovePhase => PartAnimationDriver::MovePhase,
            Self::MoveDistance => PartAnimationDriver::MoveDistance,
            Self::AttackTime => PartAnimationDriver::AttackTime,
        }
    }
}

impl PartAnimationDefBundleV1 {
    fn from_clip(clip: &PartAnimationDef) -> Self {
        match clip {
            PartAnimationDef::Loop {
                duration_secs,
                keyframes,
            } => Self::Loop {
                duration_secs: *duration_secs,
                keyframes: keyframes
                    .iter()
                    .map(PartAnimationKeyframeDefBundleV1::from_keyframe)
                    .collect(),
            },
            PartAnimationDef::Once {
                duration_secs,
                keyframes,
            } => Self::Once {
                duration_secs: *duration_secs,
                keyframes: keyframes
                    .iter()
                    .map(PartAnimationKeyframeDefBundleV1::from_keyframe)
                    .collect(),
            },
            PartAnimationDef::PingPong {
                duration_secs,
                keyframes,
            } => Self::PingPong {
                duration_secs: *duration_secs,
                keyframes: keyframes
                    .iter()
                    .map(PartAnimationKeyframeDefBundleV1::from_keyframe)
                    .collect(),
            },
            PartAnimationDef::Spin {
                axis,
                radians_per_unit,
                axis_space,
            } => Self::Spin {
                axis: [axis.x, axis.y, axis.z],
                radians_per_unit: *radians_per_unit,
                axis_space: match axis_space {
                    crate::object::registry::PartAnimationSpinAxisSpace::Join => {
                        PartAnimationSpinAxisSpaceBundleV1::Join
                    }
                    crate::object::registry::PartAnimationSpinAxisSpace::ChildLocal => {
                        PartAnimationSpinAxisSpaceBundleV1::ChildLocal
                    }
                },
            },
        }
    }

    fn to_clip(&self) -> Result<PartAnimationDef, String> {
        Ok(match self {
            Self::Loop {
                duration_secs,
                keyframes,
            } => PartAnimationDef::Loop {
                duration_secs: *duration_secs,
                keyframes: keyframes
                    .iter()
                    .map(PartAnimationKeyframeDefBundleV1::to_keyframe)
                    .collect::<Result<Vec<_>, _>>()?,
            },
            Self::Once {
                duration_secs,
                keyframes,
            } => PartAnimationDef::Once {
                duration_secs: *duration_secs,
                keyframes: keyframes
                    .iter()
                    .map(PartAnimationKeyframeDefBundleV1::to_keyframe)
                    .collect::<Result<Vec<_>, _>>()?,
            },
            Self::PingPong {
                duration_secs,
                keyframes,
            } => PartAnimationDef::PingPong {
                duration_secs: *duration_secs,
                keyframes: keyframes
                    .iter()
                    .map(PartAnimationKeyframeDefBundleV1::to_keyframe)
                    .collect::<Result<Vec<_>, _>>()?,
            },
            Self::Spin {
                axis,
                radians_per_unit,
                axis_space,
            } => PartAnimationDef::Spin {
                axis: Vec3::new(axis[0], axis[1], axis[2]),
                radians_per_unit: *radians_per_unit,
                axis_space: match axis_space {
                    PartAnimationSpinAxisSpaceBundleV1::Join => {
                        crate::object::registry::PartAnimationSpinAxisSpace::Join
                    }
                    PartAnimationSpinAxisSpaceBundleV1::ChildLocal => {
                        crate::object::registry::PartAnimationSpinAxisSpace::ChildLocal
                    }
                },
            },
        })
    }
}

impl PartAnimationKeyframeDefBundleV1 {
    fn from_keyframe(k: &PartAnimationKeyframeDef) -> Self {
        Self {
            time_secs: k.time_secs,
            delta: TransformBundleV1::from_transform(k.delta),
        }
    }

    fn to_keyframe(&self) -> Result<PartAnimationKeyframeDef, String> {
        Ok(PartAnimationKeyframeDef {
            time_secs: self.time_secs,
            delta: self.delta.to_transform(),
        })
    }
}
