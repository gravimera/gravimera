use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn deserialize_optional_nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    struct Visitor<T>(std::marker::PhantomData<T>);

    impl<'de, T> serde::de::Visitor<'de> for Visitor<T>
    where
        T: serde::Deserialize<'de>,
    {
        type Value = Option<Option<T>>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("null or a value")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(Some(None))
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(Some(None))
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let value = T::deserialize(deserializer)?;
            Ok(Some(Some(value)))
        }
    }

    deserializer.deserialize_option(Visitor(std::marker::PhantomData))
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub(crate) forward: [f32; 3],
    pub(crate) up: [f32; 3],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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
#[serde(deny_unknown_fields)]
pub(crate) struct AiPartJson {
    pub(crate) primitive: AiPrimitiveJson,
    #[serde(default)]
    pub(crate) params: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) color: Option<[f32; 4]>,
    #[serde(default)]
    pub(crate) render_priority: Option<i32>,
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
    pub(crate) color: [f32; 4],
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
    PreserveInterfaces,
    CopySource,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AiReuseAlignmentJson {
    Rotation,
    MirrorMountX,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AiReuseAlignmentFrameJson {
    Join,
    ChildAnchor,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub(crate) alignment: AiReuseAlignmentJson,
    #[serde(default)]
    pub(crate) alignment_frame: Option<AiReuseAlignmentFrameJson>,
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
    pub(crate) mobility: AiMobilityJson,
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

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AiJointKindJson {
    Hinge,
    Ball,
    // Legacy alias: older plans/prefabs used "fixed". Treat it as "free" to maximize DoF and
    // avoid surprising constraint semantics in motion authoring/validation.
    #[serde(alias = "fixed")]
    Free,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AiContactKindJson {
    Ground,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiContactStanceJson {
    pub(crate) phase_01: f32,
    pub(crate) duty_factor_01: f32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AiRotationFrameJson {
    Join,
    Parent,
    #[serde(other)]
    Unknown,
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
    /// Coordinate frame for `forward`/`up` (and `rot_quat_xyzw` when used as a rotation). When
    /// omitted, the engine assumes the attachment's JOIN FRAME for translation-only offsets. If
    /// any rotation is authored (`forward`/`up` or `rot_quat_xyzw`), this must be provided
    /// explicitly (`join` or `parent`) or conversion will fail.
    #[serde(default)]
    pub(crate) rot_frame: Option<AiRotationFrameJson>,
    // Optional quaternion rotation for convenience. When both basis vectors and a quaternion are
    // provided, the basis vectors take precedence.
    #[serde(default)]
    pub(crate) rot_quat_xyzw: Option<[f32; 4]>,
    #[serde(default)]
    pub(crate) scale: Option<[f32; 3]>,
}

fn default_vec3_array() -> [f32; 3] {
    [0.0, 0.0, 0.0]
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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
    #[serde(default, alias = "notes")]
    pub(crate) notes_text: Option<String>,
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
    TweakComponentResolvedRotWorld {
        component_id: String,
        rot: AiRotationJsonV1,
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
    TweakContact {
        component_id: String,
        contact_name: String,
        #[serde(default, deserialize_with = "deserialize_optional_nullable")]
        stance: Option<Option<AiContactStanceJson>>,
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
pub(crate) struct AiDescriptorMetaJsonV1 {
    #[serde(default)]
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) short: String,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiPromptIntentJsonV1 {
    #[serde(default)]
    pub(crate) version: u32,
    pub(crate) requires_attack: bool,
    #[serde(default)]
    pub(crate) explicit_motion_channels: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AiEditStrategyKindJsonV1 {
    DraftOpsOnly,
    PlanOpsThenDraftOps,
    PlanOpsOnly,
    Rebuild,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiEditStrategyJsonV1 {
    #[serde(default)]
    pub(crate) version: u32,
    pub(crate) strategy: AiEditStrategyKindJsonV1,
    #[serde(default)]
    pub(crate) snapshot_components: Vec<String>,
    #[serde(default)]
    pub(crate) reason: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AiMotionAuthoringDecisionJsonV1 {
    AuthorClips,
    RegenGeometryRequired,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AiAnimationDriverJsonV1 {
    Always,
    MovePhase,
    MoveDistance,
    AttackTime,
    ActionTime,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiAnimationDeltaTransformJsonV1 {
    #[serde(default)]
    pub(crate) pos: Option<[f32; 3]>,
    #[serde(default)]
    pub(crate) rot_quat_xyzw: Option<[f32; 4]>,
    #[serde(default)]
    pub(crate) scale: Option<[f32; 3]>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiAnimationKeyframeJsonV1 {
    pub(crate) t_units: f32,
    pub(crate) delta: AiAnimationDeltaTransformJsonV1,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AiSpinAxisSpaceJsonV1 {
    #[default]
    Join,
    ChildLocal,
}

impl AiSpinAxisSpaceJsonV1 {
    pub(crate) fn to_space(&self) -> crate::object::registry::PartAnimationSpinAxisSpace {
        match self {
            Self::Join => crate::object::registry::PartAnimationSpinAxisSpace::Join,
            Self::ChildLocal => crate::object::registry::PartAnimationSpinAxisSpace::ChildLocal,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AiAnimationClipJsonV1 {
    Loop {
        duration_units: f32,
        keyframes: Vec<AiAnimationKeyframeJsonV1>,
    },
    Once {
        duration_units: f32,
        keyframes: Vec<AiAnimationKeyframeJsonV1>,
    },
    PingPong {
        duration_units: f32,
        keyframes: Vec<AiAnimationKeyframeJsonV1>,
    },
    Spin {
        axis: [f32; 3],
        radians_per_unit: f32,
        #[serde(default)]
        axis_space: AiSpinAxisSpaceJsonV1,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiAuthoredAnimationSlotJsonV1 {
    pub(crate) channel: String,
    pub(crate) driver: AiAnimationDriverJsonV1,
    pub(crate) speed_scale: f32,
    pub(crate) time_offset_units: f32,
    pub(crate) clip: AiAnimationClipJsonV1,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiAuthoredAnimationEdgeJsonV1 {
    pub(crate) component: String,
    #[serde(default)]
    pub(crate) slots: Vec<AiAuthoredAnimationSlotJsonV1>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiMotionAuthoringJsonV1 {
    #[serde(default)]
    pub(crate) version: u32,
    pub(crate) applies_to: AiReviewDeltaAppliesToJsonV1,
    pub(crate) decision: AiMotionAuthoringDecisionJsonV1,
    #[serde(default)]
    pub(crate) reason: String,
    #[serde(default)]
    pub(crate) replace_channels: Vec<String>,
    #[serde(default)]
    pub(crate) edges: Vec<AiAuthoredAnimationEdgeJsonV1>,
    #[serde(default, alias = "notes")]
    pub(crate) notes_text: Option<String>,
}
