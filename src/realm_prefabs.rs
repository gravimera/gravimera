use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

use crate::object::registry::{
    AimProfile, AnchorDef, AnchorRef, AttachmentDef, ColliderProfile, MaterialKey,
    MeleeAttackProfile, MobilityDef, MobilityMode, MovementBlockRule, ObjectDef, ObjectInteraction,
    ObjectLibrary, ObjectPartDef, ObjectPartKind, PartAnimationDef, PartAnimationDriver,
    PartAnimationKeyframeDef, PartAnimationSlot, PartAnimationSpec, PrimitiveParams,
    PrimitiveVisualDef, ProjectileObstacleRule, ProjectileProfile, RangedAttackProfile,
    UnitAttackKind, UnitAttackProfile,
};

pub(crate) const PREFAB_FILE_FORMAT_VERSION: u32 = 1;

pub(crate) fn save_prefab_defs_to_dir(
    dir: &Path,
    root_prefab_id: u128,
    defs: &[ObjectDef],
) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|err| format!("Failed to create {}: {err}", dir.display()))?;

    for def in defs {
        let role = if def.object_id == root_prefab_id {
            PrefabRoleV1::Root
        } else {
            PrefabRoleV1::Internal
        };
        let doc = PrefabFileV1::from_object_def(def, role);
        let uuid = uuid::Uuid::from_u128(def.object_id).to_string();
        let path = dir.join(format!("{uuid}.json"));
        write_json_file_canonical(
            &path,
            &serde_json::to_value(doc).map_err(|err| err.to_string())?,
        )?;
    }

    Ok(())
}

pub(crate) fn load_prefabs_into_library_from_dir(
    root: &Path,
    library: &mut ObjectLibrary,
) -> Result<usize, String> {
    if !root.exists() {
        return Ok(0);
    }

    let mut loaded = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(next) = stack.pop() {
        let entries = std::fs::read_dir(&next)
            .map_err(|err| format!("Failed to list {}: {err}", next.display()))?;
        for entry in entries {
            let entry = entry.map_err(|err| format!("Failed to read dir entry: {err}"))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|v| v.to_str()) != Some("json") {
                continue;
            }
            if let Some(file_name) = path.file_name().and_then(|v| v.to_str()) {
                if file_name.ends_with(".desc.json") {
                    continue;
                }
            }
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(err) => {
                    warn!("Prefab defs: failed to read {}: {err}", path.display());
                    continue;
                }
            };
            let json: Value = match serde_json::from_slice(&bytes) {
                Ok(v) => v,
                Err(err) => {
                    warn!("Prefab defs: invalid JSON {}: {err}", path.display());
                    continue;
                }
            };
            let doc: PrefabFileV1 = match serde_json::from_value(json) {
                Ok(v) => v,
                Err(err) => {
                    warn!("Prefab defs: schema mismatch {}: {err}", path.display());
                    continue;
                }
            };
            if doc.format_version != PREFAB_FILE_FORMAT_VERSION {
                warn!(
                    "Prefab defs: ignoring {}: unsupported format_version {} (expected {}).",
                    path.display(),
                    doc.format_version,
                    PREFAB_FILE_FORMAT_VERSION
                );
                continue;
            }
            match doc.to_object_def() {
                Ok(def) => {
                    library.upsert(def);
                    loaded += 1;
                }
                Err(err) => {
                    warn!("Prefab defs: skipping {}: {err}", path.display());
                }
            }
        }
    }

    Ok(loaded)
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum PrefabRoleV1 {
    Root,
    Internal,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PrefabFileV1 {
    format_version: u32,
    prefab_id: String,
    role: PrefabRoleV1,
    label: String,
    size: Vec3Json,
    ground_origin_y: Option<f32>,
    collider: ColliderProfileJson,
    interaction: ObjectInteractionJson,
    aim: Option<AimProfileJson>,
    mobility: Option<MobilityDefJson>,
    anchors: Vec<AnchorDefJson>,
    parts: Vec<ObjectPartDefJson>,
    minimap_color_rgba: Option<ColorRgbaJson>,
    health_bar_offset_y: Option<f32>,
    projectile: Option<ProjectileProfileJson>,
    attack: Option<UnitAttackProfileJson>,
}

impl PrefabFileV1 {
    fn from_object_def(def: &ObjectDef, role: PrefabRoleV1) -> Self {
        Self {
            format_version: PREFAB_FILE_FORMAT_VERSION,
            prefab_id: uuid::Uuid::from_u128(def.object_id).to_string(),
            role,
            label: def.label.to_string(),
            size: Vec3Json::from_vec3(def.size),
            ground_origin_y: def.ground_origin_y,
            collider: ColliderProfileJson::from_profile(def.collider),
            interaction: ObjectInteractionJson::from_interaction(def.interaction),
            aim: def.aim.as_ref().map(AimProfileJson::from_aim),
            mobility: def.mobility.map(MobilityDefJson::from_mobility),
            anchors: def.anchors.iter().map(AnchorDefJson::from_anchor).collect(),
            parts: def.parts.iter().map(ObjectPartDefJson::from_part).collect(),
            minimap_color_rgba: def.minimap_color.map(ColorRgbaJson::from_color),
            health_bar_offset_y: def.health_bar_offset_y,
            projectile: def.projectile.map(ProjectileProfileJson::from_projectile),
            attack: def.attack.as_ref().map(UnitAttackProfileJson::from_attack),
        }
    }

    fn to_object_def(&self) -> Result<ObjectDef, String> {
        let prefab_uuid = uuid::Uuid::parse_str(self.prefab_id.trim())
            .map_err(|err| format!("Invalid prefab_id UUID: {err}"))?;

        let ground_origin_y = match self.ground_origin_y {
            None => None,
            Some(value) if value.is_finite() && value >= 0.0 => Some(value),
            Some(value) => {
                return Err(format!(
                    "Invalid ground_origin_y (expected finite >= 0): {value}"
                ));
            }
        };

        Ok(ObjectDef {
            object_id: prefab_uuid.as_u128(),
            label: self.label.clone().into(),
            size: self.size.to_vec3(),
            ground_origin_y,
            collider: self.collider.to_profile()?,
            interaction: self.interaction.to_interaction()?,
            aim: self.aim.as_ref().map(AimProfileJson::to_aim).transpose()?,
            mobility: self
                .mobility
                .as_ref()
                .map(MobilityDefJson::to_mobility)
                .transpose()?,
            anchors: self
                .anchors
                .iter()
                .map(AnchorDefJson::to_anchor)
                .collect::<Result<Vec<_>, _>>()?,
            parts: self
                .parts
                .iter()
                .map(ObjectPartDefJson::to_part)
                .collect::<Result<Vec<_>, _>>()?,
            minimap_color: self.minimap_color_rgba.map(ColorRgbaJson::to_color),
            health_bar_offset_y: self.health_bar_offset_y,
            enemy: None,
            muzzle: None,
            projectile: self
                .projectile
                .as_ref()
                .map(ProjectileProfileJson::to_projectile)
                .transpose()?,
            attack: self
                .attack
                .as_ref()
                .map(UnitAttackProfileJson::to_attack)
                .transpose()?,
        })
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct Vec2Json {
    x: f32,
    y: f32,
}

impl Vec2Json {
    fn from_vec2(v: Vec2) -> Self {
        Self { x: v.x, y: v.y }
    }

    fn to_vec2(self) -> Vec2 {
        Vec2::new(self.x, self.y)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct Vec3Json {
    x: f32,
    y: f32,
    z: f32,
}

impl Vec3Json {
    fn from_vec3(v: Vec3) -> Self {
        Self {
            x: v.x,
            y: v.y,
            z: v.z,
        }
    }

    fn to_vec3(self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct QuatJson {
    x: f32,
    y: f32,
    z: f32,
    w: f32,
}

impl QuatJson {
    fn from_quat(v: Quat) -> Self {
        Self {
            x: v.x,
            y: v.y,
            z: v.z,
            w: v.w,
        }
    }

    fn to_quat(self) -> Quat {
        Quat::from_xyzw(self.x, self.y, self.z, self.w)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct TransformJson {
    translation: Vec3Json,
    rotation: QuatJson,
    scale: Vec3Json,
}

impl TransformJson {
    fn from_transform(v: Transform) -> Self {
        Self {
            translation: Vec3Json::from_vec3(v.translation),
            rotation: QuatJson::from_quat(v.rotation),
            scale: Vec3Json::from_vec3(v.scale),
        }
    }

    fn to_transform(self) -> Transform {
        Transform {
            translation: self.translation.to_vec3(),
            rotation: self.rotation.to_quat(),
            scale: self.scale.to_vec3(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct ColorRgbaJson {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

impl ColorRgbaJson {
    fn from_color(color: Color) -> Self {
        let srgba = color.to_srgba();
        Self {
            r: srgba.red,
            g: srgba.green,
            b: srgba.blue,
            a: srgba.alpha,
        }
    }

    fn to_color(self) -> Color {
        Color::srgba(
            self.r.clamp(0.0, 1.0),
            self.g.clamp(0.0, 1.0),
            self.b.clamp(0.0, 1.0),
            self.a.clamp(0.0, 1.0),
        )
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ColliderProfileJson {
    None,
    CircleXz { radius: f32 },
    AabbXz { half_extents: Vec2Json },
}

impl ColliderProfileJson {
    fn from_profile(profile: ColliderProfile) -> Self {
        match profile {
            ColliderProfile::None => Self::None,
            ColliderProfile::CircleXZ { radius } => Self::CircleXz { radius },
            ColliderProfile::AabbXZ { half_extents } => Self::AabbXz {
                half_extents: Vec2Json::from_vec2(half_extents),
            },
        }
    }

    fn to_profile(self) -> Result<ColliderProfile, String> {
        Ok(match self {
            Self::None => ColliderProfile::None,
            Self::CircleXz { radius } => ColliderProfile::CircleXZ { radius },
            Self::AabbXz { half_extents } => ColliderProfile::AabbXZ {
                half_extents: half_extents.to_vec2(),
            },
        })
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum MovementBlockRuleJson {
    Always,
    UpperBodyFraction { fraction: f32 },
}

impl MovementBlockRuleJson {
    fn from_rule(rule: MovementBlockRule) -> Self {
        match rule {
            MovementBlockRule::Always => Self::Always,
            MovementBlockRule::UpperBodyFraction(fraction) => Self::UpperBodyFraction { fraction },
        }
    }

    fn to_rule(self) -> Result<MovementBlockRule, String> {
        Ok(match self {
            Self::Always => MovementBlockRule::Always,
            Self::UpperBodyFraction { fraction } => MovementBlockRule::UpperBodyFraction(fraction),
        })
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct ObjectInteractionJson {
    blocks_bullets: bool,
    blocks_laser: bool,
    movement_block: Option<MovementBlockRuleJson>,
    supports_standing: bool,
}

impl ObjectInteractionJson {
    fn from_interaction(i: ObjectInteraction) -> Self {
        Self {
            blocks_bullets: i.blocks_bullets,
            blocks_laser: i.blocks_laser,
            movement_block: i.movement_block.map(MovementBlockRuleJson::from_rule),
            supports_standing: i.supports_standing,
        }
    }

    fn to_interaction(self) -> Result<ObjectInteraction, String> {
        Ok(ObjectInteraction {
            blocks_bullets: self.blocks_bullets,
            blocks_laser: self.blocks_laser,
            movement_block: self
                .movement_block
                .map(MovementBlockRuleJson::to_rule)
                .transpose()?,
            supports_standing: self.supports_standing,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AimProfileJson {
    max_yaw_delta_degrees: Option<f32>,
    components: Vec<String>,
}

impl AimProfileJson {
    fn from_aim(aim: &AimProfile) -> Self {
        Self {
            max_yaw_delta_degrees: aim.max_yaw_delta_degrees,
            components: aim
                .components
                .iter()
                .map(|id| uuid::Uuid::from_u128(*id).to_string())
                .collect(),
        }
    }

    fn to_aim(&self) -> Result<AimProfile, String> {
        let mut components = Vec::new();
        for raw in &self.components {
            let uuid = uuid::Uuid::parse_str(raw.trim())
                .map_err(|err| format!("Invalid aim.components UUID: {err}"))?;
            components.push(uuid.as_u128());
        }
        Ok(AimProfile {
            max_yaw_delta_degrees: self.max_yaw_delta_degrees,
            components,
        })
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MobilityModeJson {
    Ground,
    Air,
}

impl MobilityModeJson {
    fn from_mode(mode: MobilityMode) -> Self {
        match mode {
            MobilityMode::Ground => Self::Ground,
            MobilityMode::Air => Self::Air,
        }
    }

    fn to_mode(self) -> MobilityMode {
        match self {
            Self::Ground => MobilityMode::Ground,
            Self::Air => MobilityMode::Air,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct MobilityDefJson {
    mode: MobilityModeJson,
    max_speed: f32,
}

impl MobilityDefJson {
    fn from_mobility(m: MobilityDef) -> Self {
        Self {
            mode: MobilityModeJson::from_mode(m.mode),
            max_speed: m.max_speed,
        }
    }

    fn to_mobility(&self) -> Result<MobilityDef, String> {
        Ok(MobilityDef {
            mode: self.mode.to_mode(),
            max_speed: self.max_speed,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AnchorDefJson {
    name: String,
    transform: TransformJson,
}

impl AnchorDefJson {
    fn from_anchor(a: &AnchorDef) -> Self {
        Self {
            name: a.name.to_string(),
            transform: TransformJson::from_transform(a.transform),
        }
    }

    fn to_anchor(&self) -> Result<AnchorDef, String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("AnchorDef.name must be non-empty".into());
        }
        Ok(AnchorDef {
            name: name.to_string().into(),
            transform: self.transform.to_transform(),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AttachmentDefJson {
    parent_anchor: String,
    child_anchor: String,
}

impl AttachmentDefJson {
    fn from_attachment(a: &AttachmentDef) -> Self {
        Self {
            parent_anchor: a.parent_anchor.to_string(),
            child_anchor: a.child_anchor.to_string(),
        }
    }

    fn to_attachment(&self) -> Result<AttachmentDef, String> {
        let parent_anchor = self.parent_anchor.trim();
        let child_anchor = self.child_anchor.trim();
        if parent_anchor.is_empty() || child_anchor.is_empty() {
            return Err("AttachmentDef anchors must be non-empty".into());
        }
        Ok(AttachmentDef {
            parent_anchor: parent_anchor.to_string().into(),
            child_anchor: child_anchor.to_string().into(),
        })
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PartAnimationDriverJson {
    Always,
    MovePhase,
    MoveDistance,
    AttackTime,
    ActionTime,
}

impl PartAnimationDriverJson {
    fn from_driver(d: PartAnimationDriver) -> Self {
        match d {
            PartAnimationDriver::Always => Self::Always,
            PartAnimationDriver::MovePhase => Self::MovePhase,
            PartAnimationDriver::MoveDistance => Self::MoveDistance,
            PartAnimationDriver::AttackTime => Self::AttackTime,
            PartAnimationDriver::ActionTime => Self::ActionTime,
        }
    }

    fn to_driver(self) -> PartAnimationDriver {
        match self {
            Self::Always => PartAnimationDriver::Always,
            Self::MovePhase => PartAnimationDriver::MovePhase,
            Self::MoveDistance => PartAnimationDriver::MoveDistance,
            Self::AttackTime => PartAnimationDriver::AttackTime,
            Self::ActionTime => PartAnimationDriver::ActionTime,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PartAnimationSpinAxisSpaceJson {
    #[default]
    Join,
    ChildLocal,
}

impl PartAnimationSpinAxisSpaceJson {
    fn from_space(space: crate::object::registry::PartAnimationSpinAxisSpace) -> Self {
        match space {
            crate::object::registry::PartAnimationSpinAxisSpace::Join => Self::Join,
            crate::object::registry::PartAnimationSpinAxisSpace::ChildLocal => Self::ChildLocal,
        }
    }

    fn to_space(self) -> crate::object::registry::PartAnimationSpinAxisSpace {
        match self {
            Self::Join => crate::object::registry::PartAnimationSpinAxisSpace::Join,
            Self::ChildLocal => crate::object::registry::PartAnimationSpinAxisSpace::ChildLocal,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum PartAnimationDefJson {
    Loop {
        duration_secs: f32,
        keyframes: Vec<PartAnimationKeyframeDefJson>,
    },
    Once {
        duration_secs: f32,
        keyframes: Vec<PartAnimationKeyframeDefJson>,
    },
    PingPong {
        duration_secs: f32,
        keyframes: Vec<PartAnimationKeyframeDefJson>,
    },
    Spin {
        axis: Vec3Json,
        radians_per_unit: f32,
        #[serde(default)]
        axis_space: PartAnimationSpinAxisSpaceJson,
    },
}

impl PartAnimationDefJson {
    fn from_clip(clip: &PartAnimationDef) -> Self {
        match clip {
            PartAnimationDef::Loop {
                duration_secs,
                keyframes,
            } => Self::Loop {
                duration_secs: *duration_secs,
                keyframes: keyframes
                    .iter()
                    .map(PartAnimationKeyframeDefJson::from_keyframe)
                    .collect(),
            },
            PartAnimationDef::Once {
                duration_secs,
                keyframes,
            } => Self::Once {
                duration_secs: *duration_secs,
                keyframes: keyframes
                    .iter()
                    .map(PartAnimationKeyframeDefJson::from_keyframe)
                    .collect(),
            },
            PartAnimationDef::PingPong {
                duration_secs,
                keyframes,
            } => Self::PingPong {
                duration_secs: *duration_secs,
                keyframes: keyframes
                    .iter()
                    .map(PartAnimationKeyframeDefJson::from_keyframe)
                    .collect(),
            },
            PartAnimationDef::Spin {
                axis,
                radians_per_unit,
                axis_space,
            } => Self::Spin {
                axis: Vec3Json::from_vec3(*axis),
                radians_per_unit: *radians_per_unit,
                axis_space: PartAnimationSpinAxisSpaceJson::from_space(*axis_space),
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
                    .map(PartAnimationKeyframeDefJson::to_keyframe)
                    .collect::<Result<Vec<_>, _>>()?,
            },
            Self::Once {
                duration_secs,
                keyframes,
            } => PartAnimationDef::Once {
                duration_secs: *duration_secs,
                keyframes: keyframes
                    .iter()
                    .map(PartAnimationKeyframeDefJson::to_keyframe)
                    .collect::<Result<Vec<_>, _>>()?,
            },
            Self::PingPong {
                duration_secs,
                keyframes,
            } => PartAnimationDef::PingPong {
                duration_secs: *duration_secs,
                keyframes: keyframes
                    .iter()
                    .map(PartAnimationKeyframeDefJson::to_keyframe)
                    .collect::<Result<Vec<_>, _>>()?,
            },
            Self::Spin {
                axis,
                radians_per_unit,
                axis_space,
            } => PartAnimationDef::Spin {
                axis: axis.to_vec3(),
                radians_per_unit: *radians_per_unit,
                axis_space: axis_space.to_space(),
            },
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PartAnimationKeyframeDefJson {
    time_secs: f32,
    delta: TransformJson,
}

impl PartAnimationKeyframeDefJson {
    fn from_keyframe(k: &PartAnimationKeyframeDef) -> Self {
        Self {
            time_secs: k.time_secs,
            delta: TransformJson::from_transform(k.delta),
        }
    }

    fn to_keyframe(&self) -> Result<PartAnimationKeyframeDef, String> {
        Ok(PartAnimationKeyframeDef {
            time_secs: self.time_secs,
            delta: self.delta.to_transform(),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PartAnimationSpecJson {
    driver: PartAnimationDriverJson,
    speed_scale: f32,
    time_offset_units: f32,
    clip: PartAnimationDefJson,
}

impl PartAnimationSpecJson {
    fn from_spec(spec: &PartAnimationSpec) -> Self {
        Self {
            driver: PartAnimationDriverJson::from_driver(spec.driver),
            speed_scale: spec.speed_scale,
            time_offset_units: spec.time_offset_units,
            clip: PartAnimationDefJson::from_clip(&spec.clip),
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

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PartAnimationSlotJson {
    channel: String,
    spec: PartAnimationSpecJson,
}

impl PartAnimationSlotJson {
    fn from_slot(slot: &PartAnimationSlot) -> Self {
        Self {
            channel: slot.channel.to_string(),
            spec: PartAnimationSpecJson::from_spec(&slot.spec),
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

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MeshKeyJson {
    UnitCube,
    UnitCylinder,
    UnitCone,
    UnitSphere,
    UnitPlane,
    UnitCapsule,
    UnitConicalFrustum,
    UnitTorus,
    UnitTriangle,
    UnitTetrahedron,
    TreeTrunk,
    TreeCone,
}

impl MeshKeyJson {
    fn from_mesh(mesh: crate::object::registry::MeshKey) -> Self {
        use crate::object::registry::MeshKey;
        match mesh {
            MeshKey::UnitCube => Self::UnitCube,
            MeshKey::UnitCylinder => Self::UnitCylinder,
            MeshKey::UnitCone => Self::UnitCone,
            MeshKey::UnitSphere => Self::UnitSphere,
            MeshKey::UnitPlane => Self::UnitPlane,
            MeshKey::UnitCapsule => Self::UnitCapsule,
            MeshKey::UnitConicalFrustum => Self::UnitConicalFrustum,
            MeshKey::UnitTorus => Self::UnitTorus,
            MeshKey::UnitTriangle => Self::UnitTriangle,
            MeshKey::UnitTetrahedron => Self::UnitTetrahedron,
            MeshKey::TreeTrunk => Self::TreeTrunk,
            MeshKey::TreeCone => Self::TreeCone,
        }
    }

    fn to_mesh(self) -> crate::object::registry::MeshKey {
        use crate::object::registry::MeshKey;
        match self {
            Self::UnitCube => MeshKey::UnitCube,
            Self::UnitCylinder => MeshKey::UnitCylinder,
            Self::UnitCone => MeshKey::UnitCone,
            Self::UnitSphere => MeshKey::UnitSphere,
            Self::UnitPlane => MeshKey::UnitPlane,
            Self::UnitCapsule => MeshKey::UnitCapsule,
            Self::UnitConicalFrustum => MeshKey::UnitConicalFrustum,
            Self::UnitTorus => MeshKey::UnitTorus,
            Self::UnitTriangle => MeshKey::UnitTriangle,
            Self::UnitTetrahedron => MeshKey::UnitTetrahedron,
            Self::TreeTrunk => MeshKey::TreeTrunk,
            Self::TreeCone => MeshKey::TreeCone,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum MaterialKeyJson {
    BuildBlock { index: usize },
    FenceStake,
    FenceStick,
    TreeTrunk { variant: usize },
    TreeMain { variant: usize },
    TreeCrown { variant: usize },
}

impl MaterialKeyJson {
    fn from_material(material: MaterialKey) -> Self {
        match material {
            MaterialKey::BuildBlock { index } => Self::BuildBlock { index },
            MaterialKey::FenceStake => Self::FenceStake,
            MaterialKey::FenceStick => Self::FenceStick,
            MaterialKey::TreeTrunk { variant } => Self::TreeTrunk { variant },
            MaterialKey::TreeMain { variant } => Self::TreeMain { variant },
            MaterialKey::TreeCrown { variant } => Self::TreeCrown { variant },
        }
    }

    fn to_material(&self) -> Result<MaterialKey, String> {
        Ok(match *self {
            Self::BuildBlock { index } => MaterialKey::BuildBlock { index },
            Self::FenceStake => MaterialKey::FenceStake,
            Self::FenceStick => MaterialKey::FenceStick,
            Self::TreeTrunk { variant } => MaterialKey::TreeTrunk { variant },
            Self::TreeMain { variant } => MaterialKey::TreeMain { variant },
            Self::TreeCrown { variant } => MaterialKey::TreeCrown { variant },
        })
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum PrimitiveParamsJson {
    Capsule {
        radius: f32,
        half_length: f32,
    },
    ConicalFrustum {
        radius_top: f32,
        radius_bottom: f32,
        height: f32,
    },
    Torus {
        minor_radius: f32,
        major_radius: f32,
    },
}

impl PrimitiveParamsJson {
    fn from_params(params: &PrimitiveParams) -> Self {
        match *params {
            PrimitiveParams::Capsule {
                radius,
                half_length,
            } => Self::Capsule {
                radius,
                half_length,
            },
            PrimitiveParams::ConicalFrustum {
                radius_top,
                radius_bottom,
                height,
            } => Self::ConicalFrustum {
                radius_top,
                radius_bottom,
                height,
            },
            PrimitiveParams::Torus {
                minor_radius,
                major_radius,
            } => Self::Torus {
                minor_radius,
                major_radius,
            },
        }
    }

    fn to_params(&self) -> PrimitiveParams {
        match *self {
            Self::Capsule {
                radius,
                half_length,
            } => PrimitiveParams::Capsule {
                radius,
                half_length,
            },
            Self::ConicalFrustum {
                radius_top,
                radius_bottom,
                height,
            } => PrimitiveParams::ConicalFrustum {
                radius_top,
                radius_bottom,
                height,
            },
            Self::Torus {
                minor_radius,
                major_radius,
            } => PrimitiveParams::Torus {
                minor_radius,
                major_radius,
            },
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum PrimitiveVisualDefJson {
    Mesh {
        mesh: MeshKeyJson,
        material: MaterialKeyJson,
    },
    Primitive {
        mesh: MeshKeyJson,
        params: Option<PrimitiveParamsJson>,
        color_rgba: ColorRgbaJson,
        unlit: bool,
    },
}

impl PrimitiveVisualDefJson {
    fn from_visual(v: &PrimitiveVisualDef) -> Self {
        match v {
            PrimitiveVisualDef::Mesh { mesh, material } => Self::Mesh {
                mesh: MeshKeyJson::from_mesh(*mesh),
                material: MaterialKeyJson::from_material(*material),
            },
            PrimitiveVisualDef::Primitive {
                mesh,
                params,
                color,
                unlit,
            } => Self::Primitive {
                mesh: MeshKeyJson::from_mesh(*mesh),
                params: params.as_ref().map(PrimitiveParamsJson::from_params),
                color_rgba: ColorRgbaJson::from_color(*color),
                unlit: *unlit,
            },
        }
    }

    fn to_visual(&self) -> Result<PrimitiveVisualDef, String> {
        Ok(match self {
            Self::Mesh { mesh, material } => PrimitiveVisualDef::Mesh {
                mesh: mesh.to_mesh(),
                material: material.to_material()?,
            },
            Self::Primitive {
                mesh,
                params,
                color_rgba,
                unlit,
            } => PrimitiveVisualDef::Primitive {
                mesh: mesh.to_mesh(),
                params: params.as_ref().map(PrimitiveParamsJson::to_params),
                color: color_rgba.to_color(),
                unlit: *unlit,
            },
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ObjectPartKindJson {
    ObjectRef { object_id: String },
    Primitive { primitive: PrimitiveVisualDefJson },
    Model { scene: String },
}

impl ObjectPartKindJson {
    fn from_kind(kind: &ObjectPartKind) -> Self {
        match kind {
            ObjectPartKind::ObjectRef { object_id } => Self::ObjectRef {
                object_id: uuid::Uuid::from_u128(*object_id).to_string(),
            },
            ObjectPartKind::Primitive { primitive } => Self::Primitive {
                primitive: PrimitiveVisualDefJson::from_visual(primitive),
            },
            ObjectPartKind::Model { scene } => Self::Model {
                scene: scene.to_string(),
            },
        }
    }

    fn to_kind(&self) -> Result<ObjectPartKind, String> {
        Ok(match self {
            Self::ObjectRef { object_id } => {
                let uuid = uuid::Uuid::parse_str(object_id.trim())
                    .map_err(|err| format!("Invalid ObjectRef object_id UUID: {err}"))?;
                ObjectPartKind::ObjectRef {
                    object_id: uuid.as_u128(),
                }
            }
            Self::Primitive { primitive } => ObjectPartKind::Primitive {
                primitive: primitive.to_visual()?,
            },
            Self::Model { scene } => ObjectPartKind::Model {
                scene: scene.clone().into(),
            },
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ObjectPartDefJson {
    part_id: Option<String>,
    #[serde(default)]
    render_priority: Option<i32>,
    kind: ObjectPartKindJson,
    attachment: Option<AttachmentDefJson>,
    animations: Vec<PartAnimationSlotJson>,
    transform: TransformJson,
}

impl ObjectPartDefJson {
    fn from_part(part: &ObjectPartDef) -> Self {
        Self {
            part_id: part.part_id.map(|id| uuid::Uuid::from_u128(id).to_string()),
            render_priority: part.render_priority,
            kind: ObjectPartKindJson::from_kind(&part.kind),
            attachment: part
                .attachment
                .as_ref()
                .map(AttachmentDefJson::from_attachment),
            animations: part
                .animations
                .iter()
                .map(PartAnimationSlotJson::from_slot)
                .collect(),
            transform: TransformJson::from_transform(part.transform),
        }
    }

    fn to_part(&self) -> Result<ObjectPartDef, String> {
        let part_id = match self
            .part_id
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            None => None,
            Some(raw) => Some(
                uuid::Uuid::parse_str(raw)
                    .map_err(|err| format!("Invalid part_id UUID: {err}"))?
                    .as_u128(),
            ),
        };

        Ok(ObjectPartDef {
            part_id,
            render_priority: self.render_priority,
            kind: self.kind.to_kind()?,
            attachment: self
                .attachment
                .as_ref()
                .map(AttachmentDefJson::to_attachment)
                .transpose()?,
            animations: self
                .animations
                .iter()
                .map(PartAnimationSlotJson::to_slot)
                .collect::<Result<Vec<_>, _>>()?,
            transform: self.transform.to_transform(),
        })
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProjectileObstacleRuleJson {
    BulletsBlockers,
    LaserBlockers,
}

impl ProjectileObstacleRuleJson {
    fn from_rule(rule: ProjectileObstacleRule) -> Self {
        match rule {
            ProjectileObstacleRule::BulletsBlockers => Self::BulletsBlockers,
            ProjectileObstacleRule::LaserBlockers => Self::LaserBlockers,
        }
    }

    fn to_rule(self) -> ProjectileObstacleRule {
        match self {
            Self::BulletsBlockers => ProjectileObstacleRule::BulletsBlockers,
            Self::LaserBlockers => ProjectileObstacleRule::LaserBlockers,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct ProjectileProfileJson {
    obstacle_rule: ProjectileObstacleRuleJson,
    speed: f32,
    ttl_secs: f32,
    damage: i32,
    spawn_energy_impact: bool,
}

impl ProjectileProfileJson {
    fn from_projectile(p: ProjectileProfile) -> Self {
        Self {
            obstacle_rule: ProjectileObstacleRuleJson::from_rule(p.obstacle_rule),
            speed: p.speed,
            ttl_secs: p.ttl_secs,
            damage: p.damage,
            spawn_energy_impact: p.spawn_energy_impact,
        }
    }

    fn to_projectile(&self) -> Result<ProjectileProfile, String> {
        Ok(ProjectileProfile {
            obstacle_rule: self.obstacle_rule.to_rule(),
            speed: self.speed,
            ttl_secs: self.ttl_secs,
            damage: self.damage,
            spawn_energy_impact: self.spawn_energy_impact,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AnchorRefJson {
    object_id: String,
    anchor: String,
}

impl AnchorRefJson {
    fn from_ref(r: &AnchorRef) -> Self {
        Self {
            object_id: uuid::Uuid::from_u128(r.object_id).to_string(),
            anchor: r.anchor.to_string(),
        }
    }

    fn to_ref(&self) -> Result<AnchorRef, String> {
        let uuid = uuid::Uuid::parse_str(self.object_id.trim())
            .map_err(|err| format!("Invalid AnchorRef object_id UUID: {err}"))?;
        let anchor = self.anchor.trim();
        if anchor.is_empty() {
            return Err("AnchorRef.anchor must be non-empty".into());
        }
        Ok(AnchorRef {
            object_id: uuid.as_u128(),
            anchor: anchor.to_string().into(),
        })
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct MeleeAttackProfileJson {
    range: f32,
    radius: f32,
    arc_degrees: f32,
}

impl MeleeAttackProfileJson {
    fn from_melee(p: MeleeAttackProfile) -> Self {
        Self {
            range: p.range,
            radius: p.radius,
            arc_degrees: p.arc_degrees,
        }
    }

    fn to_melee(&self) -> Result<MeleeAttackProfile, String> {
        Ok(MeleeAttackProfile {
            range: self.range,
            radius: self.radius,
            arc_degrees: self.arc_degrees,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RangedAttackProfileJson {
    projectile_prefab: String,
    muzzle: AnchorRefJson,
}

impl RangedAttackProfileJson {
    fn from_ranged(p: &RangedAttackProfile) -> Self {
        Self {
            projectile_prefab: uuid::Uuid::from_u128(p.projectile_prefab).to_string(),
            muzzle: AnchorRefJson::from_ref(&p.muzzle),
        }
    }

    fn to_ranged(&self) -> Result<RangedAttackProfile, String> {
        let projectile_prefab = uuid::Uuid::parse_str(self.projectile_prefab.trim())
            .map_err(|err| format!("Invalid projectile_prefab UUID: {err}"))?
            .as_u128();
        Ok(RangedAttackProfile {
            projectile_prefab,
            muzzle: self.muzzle.to_ref()?,
        })
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum UnitAttackKindJson {
    Melee,
    RangedProjectile,
}

impl UnitAttackKindJson {
    fn from_kind(kind: UnitAttackKind) -> Self {
        match kind {
            UnitAttackKind::Melee => Self::Melee,
            UnitAttackKind::RangedProjectile => Self::RangedProjectile,
        }
    }

    fn to_kind(self) -> UnitAttackKind {
        match self {
            Self::Melee => UnitAttackKind::Melee,
            Self::RangedProjectile => UnitAttackKind::RangedProjectile,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct UnitAttackProfileJson {
    kind: UnitAttackKindJson,
    cooldown_secs: f32,
    damage: i32,
    anim_window_secs: f32,
    melee: Option<MeleeAttackProfileJson>,
    ranged: Option<RangedAttackProfileJson>,
}

impl UnitAttackProfileJson {
    fn from_attack(a: &UnitAttackProfile) -> Self {
        Self {
            kind: UnitAttackKindJson::from_kind(a.kind),
            cooldown_secs: a.cooldown_secs,
            damage: a.damage,
            anim_window_secs: a.anim_window_secs,
            melee: a.melee.map(MeleeAttackProfileJson::from_melee),
            ranged: a.ranged.as_ref().map(RangedAttackProfileJson::from_ranged),
        }
    }

    fn to_attack(&self) -> Result<UnitAttackProfile, String> {
        Ok(UnitAttackProfile {
            kind: self.kind.to_kind(),
            cooldown_secs: self.cooldown_secs,
            damage: self.damage,
            anim_window_secs: self.anim_window_secs,
            melee: self
                .melee
                .as_ref()
                .map(MeleeAttackProfileJson::to_melee)
                .transpose()?,
            ranged: self
                .ranged
                .as_ref()
                .map(RangedAttackProfileJson::to_ranged)
                .transpose()?,
        })
    }
}

fn write_json_file_canonical(path: &Path, value: &Value) -> Result<(), String> {
    let Some(parent) = path.parent() else {
        return Err(format!("no parent for path {}", path.display()));
    };
    std::fs::create_dir_all(parent)
        .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;

    let bytes = canonical_json_bytes(value).map_err(|err| err.to_string())?;
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, &bytes)
        .map_err(|err| format!("Failed to write {}: {err}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, path)
        .map_err(|err| format!("Failed to rename {}: {err}", path.display()))?;

    Ok(())
}

fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, serde_json::Error> {
    let mut value = value.clone();
    canonicalize_json_value(&mut value);
    let text = serde_json::to_string_pretty(&value)?;
    Ok(format!(
        "{text}
"
    )
    .into_bytes())
}

fn canonicalize_json_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in &keys {
                if let Some(child) = map.get_mut(key) {
                    canonicalize_json_value(child);
                }
            }

            let mut sorted_keys = keys;
            sorted_keys.sort();
            let mut new_map = serde_json::Map::new();
            for key in sorted_keys {
                if let Some(value) = map.remove(&key) {
                    new_map.insert(key, value);
                }
            }
            *map = new_map;
        }
        Value::Array(items) => {
            for item in items {
                canonicalize_json_value(item);
            }
        }
        _ => {}
    }
}
