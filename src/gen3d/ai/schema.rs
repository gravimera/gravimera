use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct AiDraftJsonV1 {
    #[serde(default)]
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) collider: Option<AiColliderJson>,
    #[serde(default)]
    pub(crate) anchors: Vec<AiAnchorJson>,
    pub(crate) parts: Vec<AiPartJson>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiAnchorJson {
    pub(crate) name: String,
    pub(crate) pos: [f32; 3],
    #[serde(default)]
    pub(crate) forward: Option<[f32; 3]>,
    #[serde(default)]
    pub(crate) up: Option<[f32; 3]>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AiColliderJson {
    None,
    CircleXz {
        radius: f32,
    },
    AabbXz {
        #[serde(default)]
        half_extents: Option<[f32; 2]>,
        #[serde(default)]
        min: Option<[f32; 2]>,
        #[serde(default)]
        max: Option<[f32; 2]>,
    },
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct AiPartJson {
    pub(crate) primitive: AiPrimitiveJson,
    #[serde(default)]
    pub(crate) params: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) color: Option<[f32; 4]>,
    pub(crate) pos: [f32; 3],
    #[serde(default)]
    pub(crate) forward: Option<[f32; 3]>,
    #[serde(default)]
    pub(crate) up: Option<[f32; 3]>,
    pub(crate) scale: [f32; 3],
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AiPrimitiveJson {
    Cuboid,
    Cone,
    Cylinder,
    Sphere,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AiMobilityJson {
    Static,
    Ground { max_speed: f32 },
    Air { max_speed: f32 },
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AiProjectileObstacleRuleJson {
    BulletsBlockers,
    LaserBlockers,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AiProjectileShapeJson {
    Sphere,
    Capsule,
    Cuboid,
    Cylinder,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum AiColorInputJson {
    Rgba([f32; 4]),
    Rgb([f32; 3]),
    Text(String),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiAnchorRefJson {
    pub(crate) component: String,
    pub(crate) anchor: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiProjectileSpecJson {
    pub(crate) shape: AiProjectileShapeJson,
    #[serde(default)]
    pub(crate) radius: Option<f32>,
    #[serde(default)]
    pub(crate) length: Option<f32>,
    #[serde(default)]
    pub(crate) size: Option<[f32; 3]>,
    pub(crate) color: AiColorInputJson,
    #[serde(default)]
    pub(crate) unlit: bool,
    pub(crate) speed: f32,
    pub(crate) ttl_secs: f32,
    pub(crate) damage: i32,
    #[serde(default)]
    pub(crate) obstacle_rule: Option<AiProjectileObstacleRuleJson>,
    #[serde(default)]
    pub(crate) spawn_energy_impact: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum AiAttackJson {
    None,
    Melee {
        #[serde(default)]
        cooldown_secs: Option<f32>,
        #[serde(default)]
        damage: Option<i32>,
        #[serde(default)]
        range: Option<f32>,
        #[serde(default)]
        radius: Option<f32>,
        #[serde(default)]
        arc_degrees: Option<f32>,
    },
    RangedProjectile {
        #[serde(default)]
        cooldown_secs: Option<f32>,
        #[serde(default)]
        muzzle: Option<AiAnchorRefJson>,
        #[serde(default)]
        projectile: Option<AiProjectileSpecJson>,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiAimJson {
    #[serde(default)]
    pub(crate) max_yaw_delta_degrees: Option<f32>,
    #[serde(default)]
    pub(crate) components: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AiAnimationDriverJson {
    Always,
    MovePhase,
    MoveDistance,
    AttackTime,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AiAnimationClipJson {
    Loop {
        duration_secs: f32,
        #[serde(default)]
        keyframes: Vec<AiAnimationKeyframeJson>,
    },
    Spin {
        axis: [f32; 3],
        radians_per_unit: f32,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiAnimationSpecJson {
    pub(crate) driver: AiAnimationDriverJson,
    #[serde(default)]
    pub(crate) speed_scale: Option<f32>,
    pub(crate) clip: AiAnimationClipJson,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiRigJson {
    #[serde(default)]
    pub(crate) move_cycle_m: Option<f32>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AiReuseGroupKindJson {
    Component,
    #[serde(rename = "copy_component")]
    CopyComponent,
    Subtree,
    #[serde(rename = "copy_component_subtree")]
    CopyComponentSubtree,
    #[serde(other)]
    Unknown,
}

impl Default for AiReuseGroupKindJson {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AiReuseModeJson {
    Detached,
    Linked,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AiReuseAnchorsJson {
    PreserveTarget,
    CopySource,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct AiReuseGroupJson {
    #[serde(default)]
    pub(crate) kind: AiReuseGroupKindJson,
    #[serde(default, alias = "source_root", alias = "source_component")]
    pub(crate) source: String,
    #[serde(
        default,
        alias = "target_roots",
        alias = "target_components",
        alias = "target_component_names"
    )]
    pub(crate) targets: Vec<String>,
    #[serde(default)]
    pub(crate) mode: Option<AiReuseModeJson>,
    #[serde(default)]
    pub(crate) anchors: Option<AiReuseAnchorsJson>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiPlanJsonV1 {
    #[serde(default)]
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) rig: Option<AiRigJson>,
    #[serde(default)]
    pub(crate) mobility: Option<AiMobilityJson>,
    #[serde(default)]
    pub(crate) attack: Option<AiAttackJson>,
    #[serde(default)]
    pub(crate) aim: Option<AiAimJson>,
    #[serde(default)]
    pub(crate) collider: Option<AiColliderJson>,
    #[serde(default)]
    pub(crate) assembly_notes: String,
    #[serde(default)]
    pub(crate) root_component: Option<String>,
    #[serde(default)]
    pub(crate) reuse_groups: Vec<AiReuseGroupJson>,
    pub(crate) components: Vec<AiPlanComponentJson>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiPlanComponentJson {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) purpose: String,
    #[serde(default)]
    pub(crate) modeling_notes: String,
    #[serde(default)]
    pub(crate) size: [f32; 3],
    #[serde(default)]
    pub(crate) anchors: Vec<AiAnchorJson>,
    #[serde(default)]
    pub(crate) contacts: Vec<AiContactJson>,
    #[serde(default)]
    pub(crate) attach_to: Option<AiPlanAttachmentJson>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AiJointKindJson {
    Fixed,
    Hinge,
    Ball,
    Free,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiJointJson {
    pub(crate) kind: AiJointKindJson,
    #[serde(default)]
    pub(crate) axis_join: Option<[f32; 3]>,
    #[serde(default)]
    pub(crate) limits_degrees: Option<[f32; 2]>,
    #[serde(default)]
    pub(crate) swing_limits_degrees: Option<[f32; 2]>,
    #[serde(default)]
    pub(crate) twist_limits_degrees: Option<[f32; 2]>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AiContactKindJson {
    Ground,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiContactStanceJson {
    pub(crate) phase_01: f32,
    pub(crate) duty_factor_01: f32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiContactJson {
    pub(crate) name: String,
    pub(crate) anchor: String,
    pub(crate) kind: AiContactKindJson,
    #[serde(default)]
    pub(crate) stance: Option<AiContactStanceJson>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiPlanAttachmentJson {
    pub(crate) parent: String,
    pub(crate) parent_anchor: String,
    pub(crate) child_anchor: String,
    #[serde(default)]
    pub(crate) offset: Option<AiAttachmentOffsetJson>,
    #[serde(default)]
    pub(crate) joint: Option<AiJointJson>,
    #[serde(default)]
    pub(crate) animations: Option<BTreeMap<String, AiAnimationSpecJson>>,
    // Legacy field (plan v4) – treated as an ambient loop when present.
    #[serde(default)]
    pub(crate) animation: Option<AiPartAnimationJson>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiAttachmentOffsetJson {
    #[serde(default = "default_vec3_array")]
    pub(crate) pos: [f32; 3],
    #[serde(default)]
    pub(crate) forward: Option<[f32; 3]>,
    #[serde(default)]
    pub(crate) up: Option<[f32; 3]>,
    // Optional quaternion rotation for convenience. When both basis vectors and a quaternion are
    // provided, the basis vectors take precedence.
    #[serde(default, alias = "quat_xyzw")]
    pub(crate) rot_quat_xyzw: Option<[f32; 4]>,
    #[serde(default)]
    pub(crate) scale: Option<[f32; 3]>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiAnimationKeyframeJson {
    pub(crate) time_secs: f32,
    #[serde(default)]
    pub(crate) delta: Option<AiAttachmentOffsetJson>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AiPartAnimationJson {
    Loop {
        duration_secs: f32,
        #[serde(default)]
        keyframes: Vec<AiAnimationKeyframeJson>,
    },
}

fn default_vec3_array() -> [f32; 3] {
    [0.0, 0.0, 0.0]
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiReviewDeltaAppliesToJsonV1 {
    pub(crate) run_id: String,
    pub(crate) attempt: u32,
    pub(crate) plan_hash: String,
    pub(crate) assembly_rev: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiTransformSetJsonV1 {
    #[serde(default)]
    pub(crate) pos: Option<[f32; 3]>,
    #[serde(default)]
    pub(crate) scale: Option<[f32; 3]>,
    #[serde(default)]
    pub(crate) rot: Option<AiRotationJsonV1>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiTransformDeltaJsonV1 {
    #[serde(default)]
    pub(crate) pos: Option<[f32; 3]>,
    #[serde(default)]
    pub(crate) scale: Option<[f32; 3]>,
    #[serde(default)]
    pub(crate) rot_quat_xyzw: Option<[f32; 4]>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum AiRotationJsonV1 {
    Basis { forward: [f32; 3], up: [f32; 3] },
    Quat { quat_xyzw: [f32; 4] },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiAnchorSetJsonV1 {
    #[serde(default)]
    pub(crate) pos: Option<[f32; 3]>,
    #[serde(default)]
    pub(crate) forward: Option<[f32; 3]>,
    #[serde(default)]
    pub(crate) up: Option<[f32; 3]>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiAnchorDeltaJsonV1 {
    #[serde(default)]
    pub(crate) pos: Option<[f32; 3]>,
    #[serde(default)]
    pub(crate) rot_quat_xyzw: Option<[f32; 4]>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiAttachmentSetJsonV1 {
    pub(crate) parent_component_id: String,
    pub(crate) parent_anchor: String,
    pub(crate) child_anchor: String,
    #[serde(default)]
    pub(crate) offset: Option<AiAttachmentOffsetJson>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiReviewDeltaJsonV1 {
    #[serde(default)]
    pub(crate) version: u32,
    pub(crate) applies_to: AiReviewDeltaAppliesToJsonV1,
    pub(crate) actions: Vec<AiReviewDeltaActionJsonV1>,
    #[serde(default)]
    pub(crate) summary: Option<String>,
    #[serde(default)]
    pub(crate) notes: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct AiToolingFeedbackJsonV1 {
    #[serde(default)]
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) priority: String,
    #[serde(default)]
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) summary: String,
    #[serde(default)]
    pub(crate) details: serde_json::Value,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AiReviewDeltaActionJsonV1 {
    Accept,
    ToolingFeedback {
        feedback: AiToolingFeedbackJsonV1,
    },
    Replan {
        #[serde(default)]
        reason: String,
    },
    RegenComponent {
        component_id: String,
        #[serde(default)]
        updated_modeling_notes: String,
        #[serde(default)]
        reason: String,
    },
    TweakComponentTransform {
        component_id: String,
        #[serde(default)]
        set: Option<AiTransformSetJsonV1>,
        #[serde(default)]
        delta: Option<AiTransformDeltaJsonV1>,
        #[serde(default)]
        reason: String,
    },
    TweakAnchor {
        component_id: String,
        anchor_name: String,
        #[serde(default)]
        set: Option<AiAnchorSetJsonV1>,
        #[serde(default)]
        delta: Option<AiAnchorDeltaJsonV1>,
        #[serde(default)]
        reason: String,
    },
    TweakAttachment {
        component_id: String,
        set: AiAttachmentSetJsonV1,
        #[serde(default)]
        reason: String,
    },
    TweakAnimation {
        component_id: String,
        channel: String,
        spec: AiAnimationSpecJson,
        #[serde(default)]
        reason: String,
    },
    TweakMobility {
        mobility: AiMobilityJson,
        #[serde(default)]
        reason: String,
    },
    TweakAttack {
        attack: AiAttackJson,
        #[serde(default)]
        reason: String,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiPlanFillJsonV1 {
    #[serde(default)]
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) mobility: Option<AiMobilityJson>,
    #[serde(default)]
    pub(crate) components: Vec<AiPlanFillComponentJson>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiPlanFillComponentJson {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) animations: BTreeMap<String, AiAnimationSpecJson>,
}
