use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::object::registry::ObjectLibrary;
use crate::object::registry::{
    ObjectPartKind, PartAnimationDef, PartAnimationDriver, PartAnimationKeyframeDef,
    PartAnimationSlot, PartAnimationSpec, UnitAttackKind,
};
use crate::object::visuals::{ObjectRefEdgeBinding, PartAnimationPlayer};
use crate::prefab_descriptors::{PrefabDescriptorFileV1, PrefabDescriptorLibrary};
use crate::types::{Commandable, ObjectPrefabId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum MoveMotionAlgorithm {
    None,
    BipedWalkV1,
    QuadrupedWalkV1,
    CarWheelsV1,
    AirplanePropV1,
}

impl Default for MoveMotionAlgorithm {
    fn default() -> Self {
        Self::None
    }
}

impl MoveMotionAlgorithm {
    pub(crate) fn id_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::BipedWalkV1 => "biped_walk_v1",
            Self::QuadrupedWalkV1 => "quadruped_walk_v1",
            Self::CarWheelsV1 => "car_wheels_v1",
            Self::AirplanePropV1 => "airplane_prop_v1",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::None => "None (prefab-authored)",
            Self::BipedWalkV1 => "Biped walk (v1)",
            Self::QuadrupedWalkV1 => "Quadruped walk (v1)",
            Self::CarWheelsV1 => "Car wheels (v1)",
            Self::AirplanePropV1 => "Airplane props/rotors (v1)",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "" => None,
            "none" => Some(Self::None),
            "biped_walk_v1" => Some(Self::BipedWalkV1),
            "quadruped_walk_v1" => Some(Self::QuadrupedWalkV1),
            "car_wheels_v1" => Some(Self::CarWheelsV1),
            "airplane_prop_v1" => Some(Self::AirplanePropV1),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum IdleMotionAlgorithm {
    None,
    BipedIdleV1,
    QuadrupedIdleV1,
    CarIdleV1,
    AirplaneIdleV1,
}

impl Default for IdleMotionAlgorithm {
    fn default() -> Self {
        Self::None
    }
}

impl IdleMotionAlgorithm {
    pub(crate) fn id_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::BipedIdleV1 => "biped_idle_v1",
            Self::QuadrupedIdleV1 => "quadruped_idle_v1",
            Self::CarIdleV1 => "car_idle_v1",
            Self::AirplaneIdleV1 => "airplane_idle_v1",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::None => "None (prefab-authored)",
            Self::BipedIdleV1 => "Biped idle (v1)",
            Self::QuadrupedIdleV1 => "Quadruped idle (v1)",
            Self::CarIdleV1 => "Car idle (v1)",
            Self::AirplaneIdleV1 => "Airplane idle (v1)",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "" => None,
            "none" => Some(Self::None),
            "biped_idle_v1" => Some(Self::BipedIdleV1),
            "quadruped_idle_v1" => Some(Self::QuadrupedIdleV1),
            "car_idle_v1" => Some(Self::CarIdleV1),
            "airplane_idle_v1" => Some(Self::AirplaneIdleV1),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum AttackPrimaryMotionAlgorithm {
    None,
    BipedKickV1,
    BipedMeleeSwingV1,
    QuadrupedBiteV1,
    RangedRecoilV1,
    ToolArmDigV1,
}

impl Default for AttackPrimaryMotionAlgorithm {
    fn default() -> Self {
        Self::None
    }
}

impl AttackPrimaryMotionAlgorithm {
    pub(crate) fn id_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::BipedKickV1 => "biped_kick_v1",
            Self::BipedMeleeSwingV1 => "biped_melee_swing_v1",
            Self::QuadrupedBiteV1 => "quadruped_bite_v1",
            Self::RangedRecoilV1 => "ranged_recoil_v1",
            Self::ToolArmDigV1 => "tool_arm_dig_v1",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::None => "None (prefab-authored)",
            Self::BipedKickV1 => "Biped kick (v1)",
            Self::BipedMeleeSwingV1 => "Biped melee swing (v1)",
            Self::QuadrupedBiteV1 => "Quadruped bite (v1)",
            Self::RangedRecoilV1 => "Ranged recoil (v1)",
            Self::ToolArmDigV1 => "Tool arm dig (v1)",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "" => None,
            "none" => Some(Self::None),
            "biped_kick_v1" => Some(Self::BipedKickV1),
            "biped_melee_swing_v1" => Some(Self::BipedMeleeSwingV1),
            "quadruped_bite_v1" => Some(Self::QuadrupedBiteV1),
            "ranged_recoil_v1" => Some(Self::RangedRecoilV1),
            "tool_arm_dig_v1" => Some(Self::ToolArmDigV1),
            _ => None,
        }
    }
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MotionAlgorithmController {
    pub(crate) idle_algorithm: IdleMotionAlgorithm,
    pub(crate) move_algorithm: MoveMotionAlgorithm,
    pub(crate) attack_primary_algorithm: AttackPrimaryMotionAlgorithm,
}

impl Default for MotionAlgorithmController {
    fn default() -> Self {
        Self {
            idle_algorithm: IdleMotionAlgorithm::None,
            move_algorithm: MoveMotionAlgorithm::None,
            attack_primary_algorithm: AttackPrimaryMotionAlgorithm::None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MotionEdgeRefV1 {
    pub(crate) parent_object_id: u128,
    pub(crate) child_object_id: u128,
    pub(crate) parent_anchor: String,
    pub(crate) child_anchor: String,
}

impl MotionEdgeRefV1 {
    pub(crate) fn matches_binding(&self, binding: &ObjectRefEdgeBinding) -> bool {
        if binding.parent_object_id != self.parent_object_id {
            return false;
        }
        if binding.child_object_id != self.child_object_id {
            return false;
        }
        let Some(attachment) = binding.attachment.as_ref() else {
            return false;
        };
        attachment.parent_anchor.as_ref() == self.parent_anchor
            && attachment.child_anchor.as_ref() == self.child_anchor
    }
}

#[derive(Clone, Debug)]
pub(crate) struct BipedRigV1 {
    pub(crate) move_cycle_m: f32,
    pub(crate) walk_swing_degrees: f32,
    pub(crate) default_move_algorithm: Option<MoveMotionAlgorithm>,
    pub(crate) body: Option<MotionEdgeRefV1>,
    pub(crate) left_leg: MotionEdgeRefV1,
    pub(crate) right_leg: MotionEdgeRefV1,
    pub(crate) left_arm: Option<MotionEdgeRefV1>,
    pub(crate) right_arm: Option<MotionEdgeRefV1>,
    pub(crate) head: Option<MotionEdgeRefV1>,
    pub(crate) tail: Option<MotionEdgeRefV1>,
    pub(crate) ears: Vec<MotionEdgeRefV1>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RigSideHint {
    Left,
    Right,
}

fn rig_side_hint_from_name(name: &str) -> Option<RigSideHint> {
    let lower = name.to_ascii_lowercase();
    let mut left = false;
    let mut right = false;
    for token in lower.split(|c: char| !c.is_ascii_alphanumeric()) {
        if token.is_empty() {
            continue;
        }
        match token {
            "left" | "l" => left = true,
            "right" | "r" => right = true,
            _ => {}
        }
    }
    match (left, right) {
        (true, false) => Some(RigSideHint::Left),
        (false, true) => Some(RigSideHint::Right),
        _ => None,
    }
}

impl BipedRigV1 {
    fn normalize_left_right_hints(&mut self) {
        let left_leg_hint = rig_side_hint_from_name(self.left_leg.parent_anchor.as_ref());
        let right_leg_hint = rig_side_hint_from_name(self.right_leg.parent_anchor.as_ref());
        if left_leg_hint == Some(RigSideHint::Right) && right_leg_hint == Some(RigSideHint::Left) {
            std::mem::swap(&mut self.left_leg, &mut self.right_leg);
        }

        let left_arm_hint = self
            .left_arm
            .as_ref()
            .and_then(|edge| rig_side_hint_from_name(edge.parent_anchor.as_ref()));
        let right_arm_hint = self
            .right_arm
            .as_ref()
            .and_then(|edge| rig_side_hint_from_name(edge.parent_anchor.as_ref()));

        match (left_arm_hint, right_arm_hint) {
            (Some(RigSideHint::Right), Some(RigSideHint::Left)) => {
                std::mem::swap(&mut self.left_arm, &mut self.right_arm);
            }
            (Some(RigSideHint::Right), None) => {
                self.right_arm = self.left_arm.take();
            }
            (None, Some(RigSideHint::Left)) => {
                self.left_arm = self.right_arm.take();
            }
            _ => {}
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct QuadrupedRigV1 {
    pub(crate) move_cycle_m: f32,
    pub(crate) walk_swing_degrees: f32,
    pub(crate) default_move_algorithm: Option<MoveMotionAlgorithm>,
    pub(crate) body: Option<MotionEdgeRefV1>,
    pub(crate) front_left_leg: MotionEdgeRefV1,
    pub(crate) front_right_leg: MotionEdgeRefV1,
    pub(crate) back_left_leg: MotionEdgeRefV1,
    pub(crate) back_right_leg: MotionEdgeRefV1,
    pub(crate) head: Option<MotionEdgeRefV1>,
    pub(crate) tail: Option<MotionEdgeRefV1>,
    pub(crate) ears: Vec<MotionEdgeRefV1>,
}

#[derive(Clone, Debug)]
pub(crate) struct SpinEffectorV1 {
    pub(crate) edge: MotionEdgeRefV1,
    pub(crate) spin_axis_local: Vec3,
}

#[derive(Clone, Debug)]
pub(crate) struct ToolArmRigV1 {
    pub(crate) joints: Vec<MotionEdgeRefV1>,
}

#[derive(Clone, Debug)]
pub(crate) struct CarRigV1 {
    pub(crate) default_move_algorithm: Option<MoveMotionAlgorithm>,
    pub(crate) body: Option<MotionEdgeRefV1>,
    pub(crate) wheels: Vec<SpinEffectorV1>,
    pub(crate) tool_arms: Vec<ToolArmRigV1>,
    pub(crate) wheel_radius_m: Option<f32>,
    pub(crate) radians_per_meter: Option<f32>,
}

#[derive(Clone, Debug)]
pub(crate) struct AirplaneRigV1 {
    pub(crate) move_cycle_m: f32,
    pub(crate) default_move_algorithm: Option<MoveMotionAlgorithm>,
    pub(crate) body: Option<MotionEdgeRefV1>,
    pub(crate) propellers: Vec<SpinEffectorV1>,
    pub(crate) rotors: Vec<SpinEffectorV1>,
    pub(crate) wings: Vec<MotionEdgeRefV1>,
}

#[derive(Clone, Debug)]
pub(crate) enum MotionRigV1 {
    Biped(BipedRigV1),
    Quadruped(QuadrupedRigV1),
    Car(CarRigV1),
    Airplane(AirplaneRigV1),
}

impl MotionRigV1 {
    pub(crate) fn kind_str(&self) -> &'static str {
        match self {
            Self::Biped(_) => "biped_v1",
            Self::Quadruped(_) => "quadruped_v1",
            Self::Car(_) => "car_v1",
            Self::Airplane(_) => "airplane_v1",
        }
    }

    pub(crate) fn default_idle_algorithm(&self) -> IdleMotionAlgorithm {
        match self {
            Self::Biped(_) => IdleMotionAlgorithm::BipedIdleV1,
            Self::Quadruped(_) => IdleMotionAlgorithm::QuadrupedIdleV1,
            Self::Car(_) => IdleMotionAlgorithm::CarIdleV1,
            Self::Airplane(_) => IdleMotionAlgorithm::AirplaneIdleV1,
        }
    }

    pub(crate) fn default_move_algorithm(&self) -> MoveMotionAlgorithm {
        match self {
            Self::Biped(rig) => rig
                .default_move_algorithm
                .unwrap_or(MoveMotionAlgorithm::BipedWalkV1),
            Self::Quadruped(rig) => rig
                .default_move_algorithm
                .unwrap_or(MoveMotionAlgorithm::QuadrupedWalkV1),
            Self::Car(rig) => rig
                .default_move_algorithm
                .unwrap_or(MoveMotionAlgorithm::None),
            Self::Airplane(rig) => rig
                .default_move_algorithm
                .unwrap_or(MoveMotionAlgorithm::AirplanePropV1),
        }
    }

    pub(crate) fn applicable_idle_algorithms(&self) -> Vec<IdleMotionAlgorithm> {
        let mut out = vec![IdleMotionAlgorithm::None];
        match self {
            Self::Biped(_) => out.push(IdleMotionAlgorithm::BipedIdleV1),
            Self::Quadruped(_) => out.push(IdleMotionAlgorithm::QuadrupedIdleV1),
            Self::Car(_) => out.push(IdleMotionAlgorithm::CarIdleV1),
            Self::Airplane(_) => out.push(IdleMotionAlgorithm::AirplaneIdleV1),
        }
        out
    }

    pub(crate) fn applicable_move_algorithms(&self) -> Vec<MoveMotionAlgorithm> {
        let mut out = vec![MoveMotionAlgorithm::None];
        match self {
            Self::Biped(_) => out.push(MoveMotionAlgorithm::BipedWalkV1),
            Self::Quadruped(_) => out.push(MoveMotionAlgorithm::QuadrupedWalkV1),
            Self::Car(_) => out.push(MoveMotionAlgorithm::CarWheelsV1),
            Self::Airplane(_) => out.push(MoveMotionAlgorithm::AirplanePropV1),
        }
        out
    }

    pub(crate) fn applicable_attack_primary_algorithms(
        &self,
        attack_kind: Option<UnitAttackKind>,
    ) -> Vec<AttackPrimaryMotionAlgorithm> {
        let mut out = vec![AttackPrimaryMotionAlgorithm::None];
        match attack_kind {
            None => {
                if let Self::Car(rig) = self {
                    if !rig.tool_arms.is_empty() {
                        out.push(AttackPrimaryMotionAlgorithm::ToolArmDigV1);
                    }
                }
                return out;
            }
            Some(attack_kind) => match attack_kind {
                UnitAttackKind::RangedProjectile => {
                    out.push(AttackPrimaryMotionAlgorithm::RangedRecoilV1);
                }
                UnitAttackKind::Melee => match self {
                    Self::Biped(_) => {
                        out.push(AttackPrimaryMotionAlgorithm::BipedKickV1);
                        out.push(AttackPrimaryMotionAlgorithm::BipedMeleeSwingV1);
                    }
                    Self::Quadruped(_) => out.push(AttackPrimaryMotionAlgorithm::QuadrupedBiteV1),
                    Self::Car(rig) => {
                        if !rig.tool_arms.is_empty() {
                            out.push(AttackPrimaryMotionAlgorithm::ToolArmDigV1);
                        }
                    }
                    Self::Airplane(_) => {}
                },
            },
        }
        out
    }
}

pub(crate) fn motion_rig_v1_for_prefab(
    prefab_id: u128,
    descriptors: &PrefabDescriptorLibrary,
) -> Result<Option<MotionRigV1>, String> {
    let Some(doc) = descriptors.get(prefab_id) else {
        return Ok(None);
    };
    motion_rig_v1_from_descriptor(doc)
}

pub(crate) fn motion_rig_v1_from_descriptor(
    doc: &PrefabDescriptorFileV1,
) -> Result<Option<MotionRigV1>, String> {
    let Some(interfaces) = doc.interfaces.as_ref() else {
        return Ok(None);
    };
    let Some(value) = interfaces.extra.get("motion_rig_v1") else {
        return Ok(None);
    };
    let raw: MotionRigV1Raw = serde_json::from_value(value.clone())
        .map_err(|err| format!("motion_rig_v1 JSON schema mismatch: {err}"))?;
    MotionRigV1::try_from_raw(raw).map(Some)
}

fn uuid_str_to_u128(value: &str) -> Result<u128, String> {
    let id = uuid::Uuid::parse_str(value.trim())
        .map_err(|err| format!("Invalid UUID `{value}`: {err}"))?;
    Ok(id.as_u128())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MotionRigV1Raw {
    version: u32,
    kind: String,
    #[serde(default)]
    move_cycle_m: Option<f32>,
    #[serde(default)]
    walk_swing_degrees: Option<f32>,
    #[serde(default)]
    default_move_algorithm: Option<String>,
    #[serde(default)]
    body: Option<MotionEdgeRefV1Raw>,
    #[serde(default)]
    biped: Option<BipedRigV1Raw>,
    #[serde(default)]
    quadruped: Option<QuadrupedRigV1Raw>,
    #[serde(default)]
    car: Option<CarRigV1Raw>,
    #[serde(default)]
    airplane: Option<AirplaneRigV1Raw>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MotionEdgeRefV1Raw {
    parent_object_id: String,
    child_object_id: String,
    parent_anchor: String,
    child_anchor: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct BipedRigV1Raw {
    left_leg: MotionEdgeRefV1Raw,
    right_leg: MotionEdgeRefV1Raw,
    #[serde(default)]
    left_arm: Option<MotionEdgeRefV1Raw>,
    #[serde(default)]
    right_arm: Option<MotionEdgeRefV1Raw>,
    #[serde(default)]
    head: Option<MotionEdgeRefV1Raw>,
    #[serde(default)]
    tail: Option<MotionEdgeRefV1Raw>,
    #[serde(default)]
    ears: Vec<MotionEdgeRefV1Raw>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct QuadrupedRigV1Raw {
    front_left_leg: MotionEdgeRefV1Raw,
    front_right_leg: MotionEdgeRefV1Raw,
    back_left_leg: MotionEdgeRefV1Raw,
    back_right_leg: MotionEdgeRefV1Raw,
    #[serde(default)]
    head: Option<MotionEdgeRefV1Raw>,
    #[serde(default)]
    tail: Option<MotionEdgeRefV1Raw>,
    #[serde(default)]
    ears: Vec<MotionEdgeRefV1Raw>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SpinEffectorV1Raw {
    edge: MotionEdgeRefV1Raw,
    #[serde(default)]
    spin_axis_local: Option<[f32; 3]>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ToolArmRigV1Raw {
    joints: Vec<MotionEdgeRefV1Raw>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CarRigV1Raw {
    wheels: Vec<SpinEffectorV1Raw>,
    #[serde(default)]
    tool_arms: Vec<ToolArmRigV1Raw>,
    #[serde(default)]
    wheel_radius_m: Option<f32>,
    #[serde(default)]
    radians_per_meter: Option<f32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AirplaneRigV1Raw {
    #[serde(default)]
    propellers: Vec<SpinEffectorV1Raw>,
    #[serde(default)]
    rotors: Vec<SpinEffectorV1Raw>,
    #[serde(default)]
    wings: Vec<MotionEdgeRefV1Raw>,
}

impl MotionEdgeRefV1 {
    fn try_from_raw(raw: MotionEdgeRefV1Raw) -> Result<Self, String> {
        let parent_anchor = raw.parent_anchor.trim().to_string();
        let child_anchor = raw.child_anchor.trim().to_string();
        if parent_anchor.is_empty() || child_anchor.is_empty() {
            return Err("motion_rig_v1 edge anchors must be non-empty strings".into());
        }

        Ok(Self {
            parent_object_id: uuid_str_to_u128(&raw.parent_object_id)?,
            child_object_id: uuid_str_to_u128(&raw.child_object_id)?,
            parent_anchor,
            child_anchor,
        })
    }
}

impl MotionRigV1 {
    fn try_from_raw(raw: MotionRigV1Raw) -> Result<Self, String> {
        if raw.version != 1 {
            return Err(format!(
                "motion_rig_v1 unsupported version {} (expected 1)",
                raw.version
            ));
        }

        let default_move_algorithm = raw
            .default_move_algorithm
            .as_deref()
            .and_then(MoveMotionAlgorithm::parse);
        let default_move_algorithm = raw.default_move_algorithm.as_ref().map(|raw_value| {
            default_move_algorithm.ok_or_else(|| {
                format!(
                    "motion_rig_v1 default_move_algorithm unknown: `{}`",
                    raw_value.trim()
                )
            })
        });
        let default_move_algorithm = match default_move_algorithm {
            Some(v) => Some(v?),
            None => None,
        };

        let kind = raw.kind.trim();
        let body = raw.body.map(MotionEdgeRefV1::try_from_raw).transpose()?;
        match kind {
            "biped_v1" => {
                let biped = raw
                    .biped
                    .ok_or_else(|| "motion_rig_v1 kind=biped_v1 requires `biped`".to_string())?;
                let move_cycle_m = raw.move_cycle_m.unwrap_or(1.0).max(0.01);
                let swing = raw.walk_swing_degrees.unwrap_or(28.0).clamp(0.0, 80.0);
                let mut rig = BipedRigV1 {
                    move_cycle_m,
                    walk_swing_degrees: swing,
                    default_move_algorithm,
                    body: body.clone(),
                    left_leg: MotionEdgeRefV1::try_from_raw(biped.left_leg)?,
                    right_leg: MotionEdgeRefV1::try_from_raw(biped.right_leg)?,
                    left_arm: biped
                        .left_arm
                        .map(MotionEdgeRefV1::try_from_raw)
                        .transpose()?,
                    right_arm: biped
                        .right_arm
                        .map(MotionEdgeRefV1::try_from_raw)
                        .transpose()?,
                    head: biped.head.map(MotionEdgeRefV1::try_from_raw).transpose()?,
                    tail: biped.tail.map(MotionEdgeRefV1::try_from_raw).transpose()?,
                    ears: biped
                        .ears
                        .into_iter()
                        .map(MotionEdgeRefV1::try_from_raw)
                        .collect::<Result<Vec<_>, String>>()?,
                };
                rig.normalize_left_right_hints();
                Ok(Self::Biped(rig))
            }
            "quadruped_v1" => {
                let q = raw.quadruped.ok_or_else(|| {
                    "motion_rig_v1 kind=quadruped_v1 requires `quadruped`".to_string()
                })?;
                let move_cycle_m = raw.move_cycle_m.unwrap_or(1.0).max(0.01);
                let swing = raw.walk_swing_degrees.unwrap_or(24.0).clamp(0.0, 80.0);
                Ok(Self::Quadruped(QuadrupedRigV1 {
                    move_cycle_m,
                    walk_swing_degrees: swing,
                    default_move_algorithm,
                    body: body.clone(),
                    front_left_leg: MotionEdgeRefV1::try_from_raw(q.front_left_leg)?,
                    front_right_leg: MotionEdgeRefV1::try_from_raw(q.front_right_leg)?,
                    back_left_leg: MotionEdgeRefV1::try_from_raw(q.back_left_leg)?,
                    back_right_leg: MotionEdgeRefV1::try_from_raw(q.back_right_leg)?,
                    head: q.head.map(MotionEdgeRefV1::try_from_raw).transpose()?,
                    tail: q.tail.map(MotionEdgeRefV1::try_from_raw).transpose()?,
                    ears: q
                        .ears
                        .into_iter()
                        .map(MotionEdgeRefV1::try_from_raw)
                        .collect::<Result<Vec<_>, String>>()?,
                }))
            }
            "car_v1" => {
                let car = raw
                    .car
                    .ok_or_else(|| "motion_rig_v1 kind=car_v1 requires `car`".to_string())?;
                if car.wheels.is_empty() {
                    return Err("motion_rig_v1 car.wheels must be non-empty".into());
                }
                let wheels = car
                    .wheels
                    .into_iter()
                    .map(|wheel| {
                        let axis = wheel
                            .spin_axis_local
                            .map(|v| Vec3::new(v[0], v[1], v[2]))
                            .unwrap_or(Vec3::X);
                        let axis = if axis.length_squared() > 1e-6 && axis.is_finite() {
                            axis.normalize()
                        } else {
                            Vec3::X
                        };
                        Ok(SpinEffectorV1 {
                            edge: MotionEdgeRefV1::try_from_raw(wheel.edge)?,
                            spin_axis_local: axis,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;

                let mut tool_arms: Vec<ToolArmRigV1> = Vec::new();
                for tool_arm in car.tool_arms {
                    if tool_arm.joints.is_empty() {
                        continue;
                    }
                    tool_arms.push(ToolArmRigV1 {
                        joints: tool_arm
                            .joints
                            .into_iter()
                            .map(MotionEdgeRefV1::try_from_raw)
                            .collect::<Result<Vec<_>, String>>()?,
                    });
                }
                Ok(Self::Car(CarRigV1 {
                    default_move_algorithm,
                    body: body.clone(),
                    wheels,
                    tool_arms,
                    wheel_radius_m: car.wheel_radius_m.filter(|v| v.is_finite() && *v > 0.0),
                    radians_per_meter: car
                        .radians_per_meter
                        .filter(|v| v.is_finite() && v.abs() > 1e-6),
                }))
            }
            "airplane_v1" => {
                let airplane = raw.airplane.ok_or_else(|| {
                    "motion_rig_v1 kind=airplane_v1 requires `airplane`".to_string()
                })?;
                if airplane.propellers.is_empty() && airplane.rotors.is_empty() {
                    return Err("motion_rig_v1 airplane must declare propellers or rotors".into());
                }
                let move_cycle_m = raw.move_cycle_m.unwrap_or(1.0).max(0.01);
                let propellers = airplane
                    .propellers
                    .into_iter()
                    .map(|spinner| {
                        let axis = spinner
                            .spin_axis_local
                            .map(|v| Vec3::new(v[0], v[1], v[2]))
                            .unwrap_or(Vec3::Z);
                        let axis = if axis.length_squared() > 1e-6 && axis.is_finite() {
                            axis.normalize()
                        } else {
                            Vec3::Z
                        };
                        Ok(SpinEffectorV1 {
                            edge: MotionEdgeRefV1::try_from_raw(spinner.edge)?,
                            spin_axis_local: axis,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                let rotors = airplane
                    .rotors
                    .into_iter()
                    .map(|spinner| {
                        let axis = spinner
                            .spin_axis_local
                            .map(|v| Vec3::new(v[0], v[1], v[2]))
                            .unwrap_or(Vec3::Y);
                        let axis = if axis.length_squared() > 1e-6 && axis.is_finite() {
                            axis.normalize()
                        } else {
                            Vec3::Y
                        };
                        Ok(SpinEffectorV1 {
                            edge: MotionEdgeRefV1::try_from_raw(spinner.edge)?,
                            spin_axis_local: axis,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                let wings = airplane
                    .wings
                    .into_iter()
                    .map(MotionEdgeRefV1::try_from_raw)
                    .collect::<Result<Vec<_>, String>>()?;
                Ok(Self::Airplane(AirplaneRigV1 {
                    move_cycle_m,
                    default_move_algorithm,
                    body: body.clone(),
                    propellers,
                    rotors,
                    wings,
                }))
            }
            other => Err(format!("motion_rig_v1 unknown kind `{other}`")),
        }
    }
}

pub(crate) fn walk_swing_move_slot(
    move_cycle_m: f32,
    walk_swing_degrees: f32,
    time_offset_units: f32,
) -> PartAnimationSlot {
    swing_move_slot(Vec3::X, move_cycle_m, walk_swing_degrees, time_offset_units)
}

fn swing_move_slot(
    axis_local: Vec3,
    move_cycle_m: f32,
    swing_degrees: f32,
    time_offset_units: f32,
) -> PartAnimationSlot {
    let duration = move_cycle_m.max(0.01);
    let swing = swing_degrees.to_radians();
    let axis = if axis_local.is_finite() && axis_local.length_squared() > 1e-6 {
        axis_local.normalize()
    } else {
        Vec3::X
    };

    let keyframes = vec![
        PartAnimationKeyframeDef {
            time_secs: 0.0,
            delta: Transform::IDENTITY,
        },
        PartAnimationKeyframeDef {
            time_secs: duration * 0.25,
            delta: Transform {
                rotation: Quat::from_axis_angle(axis, swing),
                ..default()
            },
        },
        PartAnimationKeyframeDef {
            time_secs: duration * 0.50,
            delta: Transform::IDENTITY,
        },
        PartAnimationKeyframeDef {
            time_secs: duration * 0.75,
            delta: Transform {
                rotation: Quat::from_axis_angle(axis, -swing),
                ..default()
            },
        },
    ];

    PartAnimationSlot {
        channel: "move".into(),
        spec: PartAnimationSpec {
            driver: PartAnimationDriver::MovePhase,
            speed_scale: 1.0,
            time_offset_units,
            clip: PartAnimationDef::Loop {
                duration_secs: duration,
                keyframes,
            },
        },
    }
}

fn bob_move_slot(move_cycle_m: f32, amplitude_m: f32, time_offset_units: f32) -> PartAnimationSlot {
    let duration = move_cycle_m.max(0.01);
    let amp = amplitude_m.max(0.0);
    let keyframes = vec![
        PartAnimationKeyframeDef {
            time_secs: 0.0,
            delta: Transform::IDENTITY,
        },
        PartAnimationKeyframeDef {
            time_secs: duration * 0.25,
            delta: Transform {
                translation: Vec3::Y * amp,
                ..default()
            },
        },
        PartAnimationKeyframeDef {
            time_secs: duration * 0.50,
            delta: Transform::IDENTITY,
        },
        PartAnimationKeyframeDef {
            time_secs: duration * 0.75,
            delta: Transform {
                translation: Vec3::Y * amp,
                ..default()
            },
        },
    ];

    PartAnimationSlot {
        channel: "move".into(),
        spec: PartAnimationSpec {
            driver: PartAnimationDriver::MovePhase,
            speed_scale: 1.0,
            time_offset_units,
            clip: PartAnimationDef::Loop {
                duration_secs: duration,
                keyframes,
            },
        },
    }
}

pub(crate) fn wheel_spin_move_slot(axis_local: Vec3, radians_per_meter: f32) -> PartAnimationSlot {
    wheel_spin_distance_slot("move", axis_local, radians_per_meter)
}

fn wheel_spin_distance_slot(
    channel: &'static str,
    axis_local: Vec3,
    radians_per_unit: f32,
) -> PartAnimationSlot {
    PartAnimationSlot {
        channel: channel.into(),
        spec: PartAnimationSpec {
            driver: PartAnimationDriver::MoveDistance,
            speed_scale: 1.0,
            time_offset_units: 0.0,
            clip: PartAnimationDef::Spin {
                axis: axis_local,
                radians_per_unit,
            },
        },
    }
}

fn wheel_spin_ambient_slot(axis_local: Vec3, radians_per_meter: f32) -> PartAnimationSlot {
    wheel_spin_distance_slot("ambient", axis_local, radians_per_meter)
}

fn swing_idle_slot(
    axis_local: Vec3,
    duration_secs: f32,
    swing_degrees: f32,
    time_offset_secs: f32,
) -> PartAnimationSlot {
    let duration = duration_secs.max(0.01);
    let swing = swing_degrees.to_radians();
    let axis = if axis_local.is_finite() && axis_local.length_squared() > 1e-6 {
        axis_local.normalize()
    } else {
        Vec3::X
    };

    let keyframes = vec![
        PartAnimationKeyframeDef {
            time_secs: 0.0,
            delta: Transform::IDENTITY,
        },
        PartAnimationKeyframeDef {
            time_secs: duration * 0.25,
            delta: Transform {
                rotation: Quat::from_axis_angle(axis, swing),
                ..default()
            },
        },
        PartAnimationKeyframeDef {
            time_secs: duration * 0.50,
            delta: Transform::IDENTITY,
        },
        PartAnimationKeyframeDef {
            time_secs: duration * 0.75,
            delta: Transform {
                rotation: Quat::from_axis_angle(axis, -swing),
                ..default()
            },
        },
    ];

    PartAnimationSlot {
        channel: "idle".into(),
        spec: PartAnimationSpec {
            driver: PartAnimationDriver::Always,
            speed_scale: 1.0,
            time_offset_units: time_offset_secs,
            clip: PartAnimationDef::Loop {
                duration_secs: duration,
                keyframes,
            },
        },
    }
}

fn bob_idle_slot(duration_secs: f32, amplitude_m: f32, time_offset_secs: f32) -> PartAnimationSlot {
    let duration = duration_secs.max(0.01);
    let amp = amplitude_m.max(0.0);
    let keyframes = vec![
        PartAnimationKeyframeDef {
            time_secs: 0.0,
            delta: Transform::IDENTITY,
        },
        PartAnimationKeyframeDef {
            time_secs: duration * 0.25,
            delta: Transform {
                translation: Vec3::Y * amp,
                ..default()
            },
        },
        PartAnimationKeyframeDef {
            time_secs: duration * 0.50,
            delta: Transform::IDENTITY,
        },
        PartAnimationKeyframeDef {
            time_secs: duration * 0.75,
            delta: Transform {
                translation: Vec3::Y * amp,
                ..default()
            },
        },
    ];

    PartAnimationSlot {
        channel: "idle".into(),
        spec: PartAnimationSpec {
            driver: PartAnimationDriver::Always,
            speed_scale: 1.0,
            time_offset_units: time_offset_secs,
            clip: PartAnimationDef::Loop {
                duration_secs: duration,
                keyframes,
            },
        },
    }
}

fn spin_idle_slot(axis_local: Vec3, radians_per_sec: f32) -> PartAnimationSlot {
    PartAnimationSlot {
        channel: "idle".into(),
        spec: PartAnimationSpec {
            driver: PartAnimationDriver::Always,
            speed_scale: 1.0,
            time_offset_units: 0.0,
            clip: PartAnimationDef::Spin {
                axis: axis_local,
                radians_per_unit: radians_per_sec,
            },
        },
    }
}

fn swing_attack_slot(
    axis_local: Vec3,
    duration_secs: f32,
    swing_degrees: f32,
) -> PartAnimationSlot {
    let duration = duration_secs.max(0.05);
    let swing = swing_degrees.to_radians();
    let axis = if axis_local.is_finite() && axis_local.length_squared() > 1e-6 {
        axis_local.normalize()
    } else {
        Vec3::X
    };

    let keyframes = vec![
        PartAnimationKeyframeDef {
            time_secs: 0.0,
            delta: Transform::IDENTITY,
        },
        PartAnimationKeyframeDef {
            time_secs: duration * 0.20,
            delta: Transform {
                rotation: Quat::from_axis_angle(axis, -swing * 0.25),
                ..default()
            },
        },
        PartAnimationKeyframeDef {
            time_secs: duration * 0.55,
            delta: Transform {
                rotation: Quat::from_axis_angle(axis, swing),
                ..default()
            },
        },
        PartAnimationKeyframeDef {
            time_secs: duration,
            delta: Transform::IDENTITY,
        },
    ];

    PartAnimationSlot {
        channel: "attack_primary".into(),
        spec: PartAnimationSpec {
            driver: PartAnimationDriver::AttackTime,
            speed_scale: 1.0,
            time_offset_units: 0.0,
            clip: PartAnimationDef::Once {
                duration_secs: duration,
                keyframes,
            },
        },
    }
}

fn recoil_attack_slot(
    direction_local: Vec3,
    duration_secs: f32,
    distance_m: f32,
) -> PartAnimationSlot {
    let duration = duration_secs.max(0.05);
    let distance = distance_m.max(0.0);
    let dir = if direction_local.is_finite() && direction_local.length_squared() > 1e-6 {
        direction_local.normalize()
    } else {
        -Vec3::Z
    };

    let keyframes = vec![
        PartAnimationKeyframeDef {
            time_secs: 0.0,
            delta: Transform::IDENTITY,
        },
        PartAnimationKeyframeDef {
            time_secs: duration * 0.12,
            delta: Transform {
                translation: dir * distance,
                ..default()
            },
        },
        PartAnimationKeyframeDef {
            time_secs: duration * 0.35,
            delta: Transform::IDENTITY,
        },
        PartAnimationKeyframeDef {
            time_secs: duration,
            delta: Transform::IDENTITY,
        },
    ];

    PartAnimationSlot {
        channel: "attack_primary".into(),
        spec: PartAnimationSpec {
            driver: PartAnimationDriver::AttackTime,
            speed_scale: 1.0,
            time_offset_units: 0.0,
            clip: PartAnimationDef::Once {
                duration_secs: duration,
                keyframes,
            },
        },
    }
}

fn bite_attack_slot(duration_secs: f32, lunge_m: f32, pitch_degrees: f32) -> PartAnimationSlot {
    let duration = duration_secs.max(0.05);
    let lunge = lunge_m.max(0.0);
    let pitch = pitch_degrees.to_radians();

    let keyframes = vec![
        PartAnimationKeyframeDef {
            time_secs: 0.0,
            delta: Transform::IDENTITY,
        },
        PartAnimationKeyframeDef {
            time_secs: duration * 0.30,
            delta: Transform {
                translation: Vec3::Z * lunge,
                rotation: Quat::from_rotation_x(-pitch),
                ..default()
            },
        },
        PartAnimationKeyframeDef {
            time_secs: duration * 0.65,
            delta: Transform::IDENTITY,
        },
        PartAnimationKeyframeDef {
            time_secs: duration,
            delta: Transform::IDENTITY,
        },
    ];

    PartAnimationSlot {
        channel: "attack_primary".into(),
        spec: PartAnimationSpec {
            driver: PartAnimationDriver::AttackTime,
            speed_scale: 1.0,
            time_offset_units: 0.0,
            clip: PartAnimationDef::Once {
                duration_secs: duration,
                keyframes,
            },
        },
    }
}

fn prefab_has_channel_slots(library: &ObjectLibrary, root_id: u128, channel: &str) -> bool {
    use std::collections::HashSet;
    let channel = channel.trim();
    if channel.is_empty() {
        return false;
    }

    let mut visited: HashSet<u128> = HashSet::new();
    let mut stack: Vec<u128> = vec![root_id];

    while let Some(object_id) = stack.pop() {
        if !visited.insert(object_id) {
            continue;
        }
        let Some(def) = library.get(object_id) else {
            continue;
        };

        for part in def.parts.iter() {
            if part
                .animations
                .iter()
                .any(|slot| slot.channel.as_ref() == channel)
            {
                return true;
            }
            if let ObjectPartKind::ObjectRef { object_id: child } = &part.kind {
                stack.push(*child);
            }
        }
    }

    false
}

pub(crate) fn ensure_default_motion_algorithm_controllers(
    mut commands: Commands,
    library: Res<ObjectLibrary>,
    descriptors: Res<PrefabDescriptorLibrary>,
    q: Query<(Entity, &ObjectPrefabId), (With<Commandable>, Without<MotionAlgorithmController>)>,
) {
    for (entity, prefab_id) in &q {
        let rig = match motion_rig_v1_for_prefab(prefab_id.0, &descriptors) {
            Ok(Some(rig)) => rig,
            Ok(None) => continue,
            Err(err) => {
                warn!(
                    "Motion: ignoring invalid motion_rig_v1 for prefab {}: {err}",
                    uuid::Uuid::from_u128(prefab_id.0)
                );
                continue;
            }
        };

        let has_channel = |channel: &str| prefab_has_channel_slots(&library, prefab_id.0, channel);

        let mut controller = MotionAlgorithmController::default();
        if !has_channel("idle") {
            controller.idle_algorithm = rig.default_idle_algorithm();
        }
        if !has_channel("move") {
            controller.move_algorithm = rig.default_move_algorithm();
        }
        if !has_channel("attack_primary") {
            let attack_kind = library
                .get(prefab_id.0)
                .and_then(|def| def.attack.as_ref())
                .map(|a| a.kind);
            controller.attack_primary_algorithm = match attack_kind {
                Some(UnitAttackKind::RangedProjectile) => {
                    AttackPrimaryMotionAlgorithm::RangedRecoilV1
                }
                Some(UnitAttackKind::Melee) => match rig {
                    MotionRigV1::Biped(_) => AttackPrimaryMotionAlgorithm::BipedKickV1,
                    MotionRigV1::Quadruped(_) => AttackPrimaryMotionAlgorithm::QuadrupedBiteV1,
                    MotionRigV1::Car(rig) => {
                        if !rig.tool_arms.is_empty() {
                            AttackPrimaryMotionAlgorithm::ToolArmDigV1
                        } else {
                            AttackPrimaryMotionAlgorithm::None
                        }
                    }
                    MotionRigV1::Airplane(_) => AttackPrimaryMotionAlgorithm::None,
                },
                None => AttackPrimaryMotionAlgorithm::None,
            };
        }

        if controller == MotionAlgorithmController::default() {
            continue;
        }

        commands.entity(entity).insert(controller);
    }
}

pub(crate) fn apply_motion_algorithms_on_controller_change(
    mut commands: Commands,
    library: Res<ObjectLibrary>,
    descriptors: Res<PrefabDescriptorLibrary>,
    roots: Query<
        (Entity, &ObjectPrefabId, &MotionAlgorithmController),
        Changed<MotionAlgorithmController>,
    >,
    edges: Query<(Entity, &ObjectRefEdgeBinding)>,
    mut players: Query<&mut PartAnimationPlayer>,
) {
    for (root_entity, prefab_id, controller) in &roots {
        let rig = match motion_rig_v1_for_prefab(prefab_id.0, &descriptors) {
            Ok(Some(rig)) => Some(rig),
            Ok(None) => None,
            Err(err) => {
                warn!(
                    "Motion: invalid motion_rig_v1 for prefab {}: {err}",
                    uuid::Uuid::from_u128(prefab_id.0)
                );
                None
            }
        };
        apply_motion_algorithms_for_root(
            &mut commands,
            &library,
            root_entity,
            prefab_id.0,
            controller,
            rig.as_ref(),
            &edges,
            &mut players,
        );
    }
}

pub(crate) fn apply_motion_algorithms_on_new_bindings(
    mut commands: Commands,
    library: Res<ObjectLibrary>,
    descriptors: Res<PrefabDescriptorLibrary>,
    roots: Query<(&ObjectPrefabId, Option<&MotionAlgorithmController>), With<Commandable>>,
    added_edges: Query<(Entity, &ObjectRefEdgeBinding), Added<ObjectRefEdgeBinding>>,
    mut players: Query<&mut PartAnimationPlayer>,
) {
    for (edge_entity, binding) in &added_edges {
        let Ok((prefab_id, controller)) = roots.get(binding.root_entity) else {
            continue;
        };
        let Some(controller) = controller else {
            continue;
        };

        let rig = match motion_rig_v1_for_prefab(prefab_id.0, &descriptors) {
            Ok(Some(rig)) => Some(rig),
            Ok(None) => None,
            Err(err) => {
                warn!(
                    "Motion: invalid motion_rig_v1 for prefab {}: {err}",
                    uuid::Uuid::from_u128(prefab_id.0)
                );
                None
            }
        };

        let rig_ref = rig.as_ref();
        let effective = MotionAlgorithmController {
            idle_algorithm: validate_idle_algorithm(
                controller.idle_algorithm,
                rig_ref,
                prefab_id.0,
            ),
            move_algorithm: validate_move_algorithm(
                controller.move_algorithm,
                rig_ref,
                prefab_id.0,
            ),
            attack_primary_algorithm: validate_attack_primary_algorithm(
                controller.attack_primary_algorithm,
                rig_ref,
                prefab_id.0,
            ),
        };

        apply_motion_algorithms_for_edge(
            &mut commands,
            &library,
            binding.root_entity,
            &effective,
            rig_ref,
            attack_window_secs(&library, prefab_id.0),
            edge_entity,
            binding,
            &mut players,
        );
    }
}

fn validate_idle_algorithm(
    algorithm: IdleMotionAlgorithm,
    rig: Option<&MotionRigV1>,
    prefab_id: u128,
) -> IdleMotionAlgorithm {
    match (algorithm, rig) {
        (IdleMotionAlgorithm::None, _) => IdleMotionAlgorithm::None,
        (IdleMotionAlgorithm::BipedIdleV1, Some(MotionRigV1::Biped(_))) => algorithm,
        (IdleMotionAlgorithm::QuadrupedIdleV1, Some(MotionRigV1::Quadruped(_))) => algorithm,
        (IdleMotionAlgorithm::CarIdleV1, Some(MotionRigV1::Car(_))) => algorithm,
        (IdleMotionAlgorithm::AirplaneIdleV1, Some(MotionRigV1::Airplane(_))) => algorithm,
        (alg, Some(rig)) => {
            warn!(
                "Motion: ignoring idle algorithm {} for rig kind {} (prefab {})",
                alg.id_str(),
                rig.kind_str(),
                uuid::Uuid::from_u128(prefab_id)
            );
            IdleMotionAlgorithm::None
        }
        (alg, None) => {
            warn!(
                "Motion: ignoring idle algorithm {} because prefab {} has no motion_rig_v1",
                alg.id_str(),
                uuid::Uuid::from_u128(prefab_id)
            );
            IdleMotionAlgorithm::None
        }
    }
}

fn validate_move_algorithm(
    algorithm: MoveMotionAlgorithm,
    rig: Option<&MotionRigV1>,
    prefab_id: u128,
) -> MoveMotionAlgorithm {
    match (algorithm, rig) {
        (MoveMotionAlgorithm::None, _) => MoveMotionAlgorithm::None,
        (MoveMotionAlgorithm::BipedWalkV1, Some(MotionRigV1::Biped(_))) => algorithm,
        (MoveMotionAlgorithm::QuadrupedWalkV1, Some(MotionRigV1::Quadruped(_))) => algorithm,
        (MoveMotionAlgorithm::CarWheelsV1, Some(MotionRigV1::Car(_))) => algorithm,
        (MoveMotionAlgorithm::AirplanePropV1, Some(MotionRigV1::Airplane(_))) => algorithm,
        (alg, Some(rig)) => {
            warn!(
                "Motion: ignoring move algorithm {} for rig kind {} (prefab {})",
                alg.id_str(),
                rig.kind_str(),
                uuid::Uuid::from_u128(prefab_id)
            );
            MoveMotionAlgorithm::None
        }
        (alg, None) => {
            warn!(
                "Motion: ignoring move algorithm {} because prefab {} has no motion_rig_v1",
                alg.id_str(),
                uuid::Uuid::from_u128(prefab_id)
            );
            MoveMotionAlgorithm::None
        }
    }
}

fn validate_attack_primary_algorithm(
    algorithm: AttackPrimaryMotionAlgorithm,
    rig: Option<&MotionRigV1>,
    prefab_id: u128,
) -> AttackPrimaryMotionAlgorithm {
    match (algorithm, rig) {
        (AttackPrimaryMotionAlgorithm::None, _) => AttackPrimaryMotionAlgorithm::None,
        (AttackPrimaryMotionAlgorithm::RangedRecoilV1, Some(_)) => algorithm,
        (AttackPrimaryMotionAlgorithm::BipedKickV1, Some(MotionRigV1::Biped(_))) => algorithm,
        (AttackPrimaryMotionAlgorithm::BipedMeleeSwingV1, Some(MotionRigV1::Biped(_))) => algorithm,
        (AttackPrimaryMotionAlgorithm::QuadrupedBiteV1, Some(MotionRigV1::Quadruped(_))) => {
            algorithm
        }
        (AttackPrimaryMotionAlgorithm::ToolArmDigV1, Some(MotionRigV1::Car(rig)))
            if !rig.tool_arms.is_empty() =>
        {
            algorithm
        }
        (alg, Some(rig)) => {
            warn!(
                "Motion: ignoring attack algorithm {} for rig kind {} (prefab {})",
                alg.id_str(),
                rig.kind_str(),
                uuid::Uuid::from_u128(prefab_id)
            );
            AttackPrimaryMotionAlgorithm::None
        }
        (alg, None) => {
            warn!(
                "Motion: ignoring attack algorithm {} because prefab {} has no motion_rig_v1",
                alg.id_str(),
                uuid::Uuid::from_u128(prefab_id)
            );
            AttackPrimaryMotionAlgorithm::None
        }
    }
}

fn attack_window_secs(library: &ObjectLibrary, prefab_id: u128) -> f32 {
    library
        .get(prefab_id)
        .and_then(|def| def.attack.as_ref())
        .map(|attack| attack.anim_window_secs)
        .unwrap_or(0.35)
        .max(0.05)
}

fn apply_motion_algorithms_for_root(
    commands: &mut Commands,
    library: &ObjectLibrary,
    root_entity: Entity,
    prefab_id: u128,
    controller: &MotionAlgorithmController,
    rig: Option<&MotionRigV1>,
    edges: &Query<(Entity, &ObjectRefEdgeBinding)>,
    players: &mut Query<&mut PartAnimationPlayer>,
) {
    let idle_algorithm = validate_idle_algorithm(controller.idle_algorithm, rig, prefab_id);
    let move_algorithm = validate_move_algorithm(controller.move_algorithm, rig, prefab_id);
    let attack_algorithm =
        validate_attack_primary_algorithm(controller.attack_primary_algorithm, rig, prefab_id);
    let attack_window_secs = attack_window_secs(library, prefab_id);

    for (edge_entity, binding) in edges.iter() {
        if binding.root_entity != root_entity {
            continue;
        }

        apply_motion_algorithms_for_edge(
            commands,
            library,
            root_entity,
            &MotionAlgorithmController {
                idle_algorithm,
                move_algorithm,
                attack_primary_algorithm: attack_algorithm,
            },
            rig,
            attack_window_secs,
            edge_entity,
            binding,
            players,
        );
    }
}

fn apply_motion_algorithms_for_edge(
    commands: &mut Commands,
    library: &ObjectLibrary,
    root_entity: Entity,
    controller: &MotionAlgorithmController,
    rig: Option<&MotionRigV1>,
    attack_window_secs: f32,
    edge_entity: Entity,
    binding: &ObjectRefEdgeBinding,
    players: &mut Query<&mut PartAnimationPlayer>,
) {
    let idle_override_slot = match (controller.idle_algorithm, rig.as_ref()) {
        (IdleMotionAlgorithm::None, _) => None,
        (IdleMotionAlgorithm::BipedIdleV1, Some(MotionRigV1::Biped(rig))) => {
            biped_idle_override_for_binding(rig, binding, library)
        }
        (IdleMotionAlgorithm::QuadrupedIdleV1, Some(MotionRigV1::Quadruped(rig))) => {
            quadruped_idle_override_for_binding(rig, binding, library)
        }
        (IdleMotionAlgorithm::CarIdleV1, Some(MotionRigV1::Car(rig))) => {
            car_idle_override_for_binding(rig, binding, library)
        }
        (IdleMotionAlgorithm::AirplaneIdleV1, Some(MotionRigV1::Airplane(rig))) => {
            airplane_idle_override_for_binding(rig, binding, library)
        }
        (_, _) => None,
    };

    let move_override_slot = match (controller.move_algorithm, rig.as_ref()) {
        (MoveMotionAlgorithm::None, _) => None,
        (MoveMotionAlgorithm::BipedWalkV1, Some(MotionRigV1::Biped(rig))) => {
            biped_walk_override_for_binding(rig, binding, library)
        }
        (MoveMotionAlgorithm::QuadrupedWalkV1, Some(MotionRigV1::Quadruped(rig))) => {
            quadruped_walk_override_for_binding(rig, binding, library)
        }
        (MoveMotionAlgorithm::CarWheelsV1, Some(MotionRigV1::Car(rig))) => {
            car_wheels_override_for_binding(rig, binding, library)
        }
        (MoveMotionAlgorithm::AirplanePropV1, Some(MotionRigV1::Airplane(rig))) => {
            airplane_prop_override_for_binding(rig, binding, library)
        }
        (_, _) => None,
    };

    let attack_override_slots: Vec<PartAnimationSlot> =
        match (controller.attack_primary_algorithm, rig.as_ref()) {
            (AttackPrimaryMotionAlgorithm::None, _) => Vec::new(),
            (AttackPrimaryMotionAlgorithm::RangedRecoilV1, Some(rig)) => {
                ranged_recoil_override_for_binding(rig, binding, library, attack_window_secs)
                    .into_iter()
                    .collect()
            }
            (AttackPrimaryMotionAlgorithm::BipedKickV1, Some(MotionRigV1::Biped(rig))) => {
                biped_kick_override_for_binding(rig, binding, library, attack_window_secs)
            }
            (AttackPrimaryMotionAlgorithm::BipedMeleeSwingV1, Some(MotionRigV1::Biped(rig))) => {
                biped_melee_swing_override_for_binding(rig, binding, library, attack_window_secs)
            }
            (AttackPrimaryMotionAlgorithm::QuadrupedBiteV1, Some(MotionRigV1::Quadruped(rig))) => {
                quadruped_bite_override_for_binding(rig, binding, library, attack_window_secs)
                    .into_iter()
                    .collect()
            }
            (AttackPrimaryMotionAlgorithm::ToolArmDigV1, Some(MotionRigV1::Car(rig))) => {
                tool_arm_dig_override_for_binding(rig, binding, library, attack_window_secs)
                    .into_iter()
                    .collect()
            }
            (_, _) => Vec::new(),
        };

    let mut effective_slots = binding.base_slots.clone();
    if let Some(slot) = idle_override_slot {
        effective_slots.retain(|s| s.channel.as_ref() != "idle");
        effective_slots.push(slot);
    }
    if let Some(slot) = move_override_slot {
        effective_slots.retain(|s| s.channel.as_ref() != "move");
        if slot.channel.as_ref() != "move" {
            effective_slots.retain(|s| s.channel.as_ref() != slot.channel.as_ref());
            if slot.channel.as_ref() == "ambient" {
                // A MoveDistance-driven ambient spin is a persistent pose override; remove any
                // idle slots so wheels don't snap back when the unit stops moving.
                effective_slots.retain(|s| s.channel.as_ref() != "idle");
            }
        }
        effective_slots.push(slot);
    }
    if !attack_override_slots.is_empty() {
        effective_slots.retain(|s| s.channel.as_ref() != "attack_primary");
        effective_slots.extend(attack_override_slots);
    }

    let should_have_player = binding.apply_aim_yaw || !effective_slots.is_empty();
    if !should_have_player {
        commands
            .entity(edge_entity)
            .insert(binding_base_pose_transform(binding, library));
        commands.entity(edge_entity).remove::<PartAnimationPlayer>();
        return;
    }

    let updated = PartAnimationPlayer {
        root_entity,
        parent_object_id: binding.parent_object_id,
        child_object_id: Some(binding.child_object_id),
        attachment: binding.attachment.clone(),
        base_transform: binding.base_transform,
        animations: effective_slots,
        apply_aim_yaw: binding.apply_aim_yaw,
    };

    if let Ok(mut player) = players.get_mut(edge_entity) {
        *player = updated;
    } else {
        commands.entity(edge_entity).insert(updated);
    }
}

fn biped_walk_override_for_binding(
    rig: &BipedRigV1,
    binding: &ObjectRefEdgeBinding,
    library: &ObjectLibrary,
) -> Option<PartAnimationSlot> {
    let cycle = rig.move_cycle_m;
    let swing = rig.walk_swing_degrees;

    if rig.left_leg.matches_binding(binding) {
        return Some(walk_swing_move_slot(cycle, swing, 0.0));
    }
    if rig.right_leg.matches_binding(binding) {
        return Some(walk_swing_move_slot(cycle, swing, cycle * 0.5));
    }

    if let Some(body) = rig.body.as_ref() {
        if body.matches_binding(binding) {
            let size = library.size(binding.child_object_id).unwrap_or(Vec3::ONE);
            let scale = binding.base_transform.scale.abs();
            let effective = (size * scale).abs().max(Vec3::splat(0.01));
            let amp = (effective.y * 0.02).clamp(0.0025, 0.06);
            return Some(bob_move_slot(cycle, amp, 0.0));
        }
    }

    if let Some(left_arm) = rig.left_arm.as_ref() {
        if left_arm.matches_binding(binding) {
            return Some(walk_swing_move_slot(cycle, swing * 0.7, cycle * 0.5));
        }
    }
    if let Some(right_arm) = rig.right_arm.as_ref() {
        if right_arm.matches_binding(binding) {
            return Some(walk_swing_move_slot(cycle, swing * 0.7, 0.0));
        }
    }

    if let Some(head) = rig.head.as_ref() {
        if head.matches_binding(binding) {
            return Some(swing_move_slot(Vec3::X, cycle, swing * 0.25, cycle * 0.25));
        }
    }

    if let Some(tail) = rig.tail.as_ref() {
        if tail.matches_binding(binding) {
            return Some(swing_move_slot(Vec3::Y, cycle, swing * 0.35, cycle * 0.25));
        }
    }

    if !rig.ears.is_empty() {
        for (idx, ear) in rig.ears.iter().enumerate() {
            if ear.matches_binding(binding) {
                let phase = if idx % 2 == 0 { 0.0 } else { cycle * 0.5 };
                return Some(swing_move_slot(Vec3::X, cycle, swing * 0.20, phase));
            }
        }
    }
    None
}

fn quadruped_walk_override_for_binding(
    rig: &QuadrupedRigV1,
    binding: &ObjectRefEdgeBinding,
    library: &ObjectLibrary,
) -> Option<PartAnimationSlot> {
    let cycle = rig.move_cycle_m;
    let swing = rig.walk_swing_degrees;

    if rig.front_left_leg.matches_binding(binding) || rig.back_right_leg.matches_binding(binding) {
        return Some(walk_swing_move_slot(cycle, swing, 0.0));
    }
    if rig.front_right_leg.matches_binding(binding) || rig.back_left_leg.matches_binding(binding) {
        return Some(walk_swing_move_slot(cycle, swing, cycle * 0.5));
    }

    if let Some(body) = rig.body.as_ref() {
        if body.matches_binding(binding) {
            let size = library.size(binding.child_object_id).unwrap_or(Vec3::ONE);
            let scale = binding.base_transform.scale.abs();
            let effective = (size * scale).abs().max(Vec3::splat(0.01));
            let amp = (effective.y * 0.02).clamp(0.0025, 0.06);
            return Some(bob_move_slot(cycle, amp, 0.0));
        }
    }

    if let Some(head) = rig.head.as_ref() {
        if head.matches_binding(binding) {
            return Some(swing_move_slot(Vec3::X, cycle, swing * 0.22, cycle * 0.25));
        }
    }

    if let Some(tail) = rig.tail.as_ref() {
        if tail.matches_binding(binding) {
            return Some(swing_move_slot(Vec3::Y, cycle, swing * 0.30, cycle * 0.25));
        }
    }

    if !rig.ears.is_empty() {
        for (idx, ear) in rig.ears.iter().enumerate() {
            if ear.matches_binding(binding) {
                let phase = if idx % 2 == 0 { 0.0 } else { cycle * 0.5 };
                return Some(swing_move_slot(Vec3::X, cycle, swing * 0.18, phase));
            }
        }
    }
    None
}

fn car_wheels_override_for_binding(
    rig: &CarRigV1,
    binding: &ObjectRefEdgeBinding,
    library: &ObjectLibrary,
) -> Option<PartAnimationSlot> {
    fn spin_axis_from_effective_size(effective_size: Vec3) -> Vec3 {
        let e = effective_size.abs();
        if !e.is_finite() {
            return Vec3::X;
        }
        if e.x <= e.y && e.x <= e.z {
            Vec3::X
        } else if e.y <= e.x && e.y <= e.z {
            Vec3::Y
        } else {
            Vec3::Z
        }
    }

    fn radius_from_effective_size_and_axis(effective: Vec3, axis: Vec3) -> f32 {
        let effective = effective.abs();
        let axis = axis.abs();
        if axis.x >= axis.y && axis.x >= axis.z {
            0.5 * effective.y.max(effective.z).max(0.01)
        } else if axis.y >= axis.x && axis.y >= axis.z {
            0.5 * effective.x.max(effective.z).max(0.01)
        } else {
            0.5 * effective.x.max(effective.y).max(0.01)
        }
    }

    fn roll_sign_for_base_transform(axis_local: Vec3, base: Transform) -> f32 {
        if !axis_local.is_finite() {
            return 1.0;
        }

        let axis_parent = base.rotation * (axis_local * base.scale);
        if !axis_parent.is_finite() || axis_parent.length_squared() <= 1e-6 {
            return 1.0;
        }

        let roll_dir = axis_parent.cross(Vec3::Y);
        if !roll_dir.is_finite() || roll_dir.length_squared() <= 1e-6 {
            return 1.0;
        }

        if roll_dir.normalize().dot(Vec3::Z) < 0.0 {
            -1.0
        } else {
            1.0
        }
    }

    if let Some(body) = rig.body.as_ref() {
        if body.matches_binding(binding) {
            let size = library.size(binding.child_object_id).unwrap_or(Vec3::ONE);
            let scale = binding.base_transform.scale.abs();
            let effective = (size * scale).abs().max(Vec3::splat(0.01));
            let amp = (effective.y * 0.01).clamp(0.0015, 0.04);
            return Some(bob_move_slot(1.0, amp, 0.0));
        }
    }

    let wheel = rig
        .wheels
        .iter()
        .find(|w| w.edge.matches_binding(binding))?;

    let size = library.size(binding.child_object_id).unwrap_or(Vec3::ONE);
    let scale = binding.base_transform.scale.abs();
    let effective = (size * scale).abs().max(Vec3::splat(0.01));

    // The spin axis in authored motion rigs isn't always reliable. Prefer a geometry-derived
    // axis for wheels so roll uses the axle/thickness direction (smallest extent).
    let axis_local = if effective.is_finite() {
        spin_axis_from_effective_size(effective)
    } else {
        wheel.spin_axis_local
    };

    let radians_per_meter = if let Some(v) = rig.radians_per_meter {
        v
    } else if let Some(radius_m) = rig.wheel_radius_m {
        1.0 / radius_m.max(0.01)
    } else {
        let radius = radius_from_effective_size_and_axis(effective, axis_local);
        (1.0 / radius.max(0.01)).clamp(-200.0, 200.0)
    };

    let sign = roll_sign_for_base_transform(axis_local, binding.base_transform);
    Some(wheel_spin_ambient_slot(
        axis_local,
        radians_per_meter * sign,
    ))
}

fn airplane_prop_override_for_binding(
    rig: &AirplaneRigV1,
    binding: &ObjectRefEdgeBinding,
    library: &ObjectLibrary,
) -> Option<PartAnimationSlot> {
    const PROPELLER_MULTIPLIER: f32 = 20.0;
    const ROTOR_MULTIPLIER: f32 = 12.0;

    if let Some(body) = rig.body.as_ref() {
        if body.matches_binding(binding) {
            let size = library.size(binding.child_object_id).unwrap_or(Vec3::ONE);
            let scale = binding.base_transform.scale.abs();
            let effective = (size * scale).abs().max(Vec3::splat(0.01));
            let amp = (effective.y * 0.015).clamp(0.0025, 0.05);
            return Some(bob_move_slot(rig.move_cycle_m, amp, 0.0));
        }
    }

    if !rig.wings.is_empty() {
        for (idx, wing) in rig.wings.iter().enumerate() {
            if wing.matches_binding(binding) {
                let phase = if idx % 2 == 0 {
                    0.0
                } else {
                    rig.move_cycle_m * 0.5
                };
                return Some(swing_move_slot(Vec3::X, rig.move_cycle_m, 15.0, phase));
            }
        }
    }

    fn radius_from_effective_size_and_axis(effective: Vec3, axis: Vec3) -> f32 {
        let axis = axis.abs();
        if axis.x >= axis.y && axis.x >= axis.z {
            0.5 * effective.y.max(effective.z).max(0.01)
        } else if axis.y >= axis.x && axis.y >= axis.z {
            0.5 * effective.x.max(effective.z).max(0.01)
        } else {
            0.5 * effective.x.max(effective.y).max(0.01)
        }
    }

    let size = library.size(binding.child_object_id).unwrap_or(Vec3::ONE);
    let scale = binding.base_transform.scale.abs();
    let effective = (size * scale).abs().max(Vec3::splat(0.01));

    if let Some(spinner) = rig
        .propellers
        .iter()
        .find(|p| p.edge.matches_binding(binding))
    {
        let radius = radius_from_effective_size_and_axis(effective, spinner.spin_axis_local);
        let radians_per_meter = (PROPELLER_MULTIPLIER / radius).clamp(-1200.0, 1200.0);
        return Some(wheel_spin_move_slot(
            spinner.spin_axis_local,
            radians_per_meter,
        ));
    }

    if let Some(spinner) = rig.rotors.iter().find(|p| p.edge.matches_binding(binding)) {
        let radius = radius_from_effective_size_and_axis(effective, spinner.spin_axis_local);
        let radians_per_meter = (ROTOR_MULTIPLIER / radius).clamp(-1200.0, 1200.0);
        return Some(wheel_spin_move_slot(
            spinner.spin_axis_local,
            radians_per_meter,
        ));
    }

    None
}

fn biped_idle_override_for_binding(
    rig: &BipedRigV1,
    binding: &ObjectRefEdgeBinding,
    library: &ObjectLibrary,
) -> Option<PartAnimationSlot> {
    const BODY_PERIOD_SECS: f32 = 2.4;
    const LIMB_PERIOD_SECS: f32 = 3.2;

    if let Some(body) = rig.body.as_ref() {
        if body.matches_binding(binding) {
            let size = library.size(binding.child_object_id).unwrap_or(Vec3::ONE);
            let scale = binding.base_transform.scale.abs();
            let effective = (size * scale).abs().max(Vec3::splat(0.01));
            let amp = (effective.y * 0.01).clamp(0.0020, 0.04);
            return Some(bob_idle_slot(BODY_PERIOD_SECS, amp, 0.0));
        }
    }

    if let Some(head) = rig.head.as_ref() {
        if head.matches_binding(binding) {
            return Some(swing_idle_slot(
                Vec3::X,
                LIMB_PERIOD_SECS,
                8.0,
                LIMB_PERIOD_SECS * 0.15,
            ));
        }
    }

    if let Some(tail) = rig.tail.as_ref() {
        if tail.matches_binding(binding) {
            return Some(swing_idle_slot(
                Vec3::Y,
                LIMB_PERIOD_SECS,
                12.0,
                LIMB_PERIOD_SECS * 0.35,
            ));
        }
    }

    if let Some(left_arm) = rig.left_arm.as_ref() {
        if left_arm.matches_binding(binding) {
            return Some(swing_idle_slot(
                Vec3::X,
                LIMB_PERIOD_SECS,
                6.0,
                LIMB_PERIOD_SECS * 0.50,
            ));
        }
    }
    if let Some(right_arm) = rig.right_arm.as_ref() {
        if right_arm.matches_binding(binding) {
            return Some(swing_idle_slot(Vec3::X, LIMB_PERIOD_SECS, 6.0, 0.0));
        }
    }

    if !rig.ears.is_empty() {
        for (idx, ear) in rig.ears.iter().enumerate() {
            if ear.matches_binding(binding) {
                let phase = (idx as f32) * 0.37;
                return Some(swing_idle_slot(Vec3::X, 1.8, 10.0, phase));
            }
        }
    }

    None
}

fn quadruped_idle_override_for_binding(
    rig: &QuadrupedRigV1,
    binding: &ObjectRefEdgeBinding,
    library: &ObjectLibrary,
) -> Option<PartAnimationSlot> {
    const BODY_PERIOD_SECS: f32 = 2.2;
    const LIMB_PERIOD_SECS: f32 = 3.0;

    if let Some(body) = rig.body.as_ref() {
        if body.matches_binding(binding) {
            let size = library.size(binding.child_object_id).unwrap_or(Vec3::ONE);
            let scale = binding.base_transform.scale.abs();
            let effective = (size * scale).abs().max(Vec3::splat(0.01));
            let amp = (effective.y * 0.012).clamp(0.0020, 0.04);
            return Some(bob_idle_slot(BODY_PERIOD_SECS, amp, 0.0));
        }
    }

    if let Some(head) = rig.head.as_ref() {
        if head.matches_binding(binding) {
            return Some(swing_idle_slot(
                Vec3::X,
                LIMB_PERIOD_SECS,
                10.0,
                LIMB_PERIOD_SECS * 0.10,
            ));
        }
    }

    if let Some(tail) = rig.tail.as_ref() {
        if tail.matches_binding(binding) {
            return Some(swing_idle_slot(
                Vec3::Y,
                LIMB_PERIOD_SECS,
                14.0,
                LIMB_PERIOD_SECS * 0.25,
            ));
        }
    }

    if !rig.ears.is_empty() {
        for (idx, ear) in rig.ears.iter().enumerate() {
            if ear.matches_binding(binding) {
                let phase = (idx as f32) * 0.41;
                return Some(swing_idle_slot(Vec3::X, 1.7, 12.0, phase));
            }
        }
    }

    None
}

fn car_idle_override_for_binding(
    rig: &CarRigV1,
    binding: &ObjectRefEdgeBinding,
    library: &ObjectLibrary,
) -> Option<PartAnimationSlot> {
    if let Some(body) = rig.body.as_ref() {
        if body.matches_binding(binding) {
            let size = library.size(binding.child_object_id).unwrap_or(Vec3::ONE);
            let scale = binding.base_transform.scale.abs();
            let effective = (size * scale).abs().max(Vec3::splat(0.01));
            let amp = (effective.y * 0.006).clamp(0.0015, 0.03);
            return Some(bob_idle_slot(1.4, amp, 0.0));
        }
    }
    None
}

fn airplane_idle_override_for_binding(
    rig: &AirplaneRigV1,
    binding: &ObjectRefEdgeBinding,
    library: &ObjectLibrary,
) -> Option<PartAnimationSlot> {
    const PROPELLER_RAD_PER_SEC: f32 = 18.0;
    const ROTOR_RAD_PER_SEC: f32 = 12.0;

    if let Some(body) = rig.body.as_ref() {
        if body.matches_binding(binding) {
            let size = library.size(binding.child_object_id).unwrap_or(Vec3::ONE);
            let scale = binding.base_transform.scale.abs();
            let effective = (size * scale).abs().max(Vec3::splat(0.01));
            let amp = (effective.y * 0.008).clamp(0.0020, 0.04);
            return Some(bob_idle_slot(1.9, amp, 0.0));
        }
    }

    if !rig.wings.is_empty() {
        for (idx, wing) in rig.wings.iter().enumerate() {
            if wing.matches_binding(binding) {
                let phase = if idx % 2 == 0 { 0.0 } else { 0.6 };
                return Some(swing_idle_slot(Vec3::X, 2.6, 6.0, phase));
            }
        }
    }

    let size = library.size(binding.child_object_id).unwrap_or(Vec3::ONE);
    let scale = binding.base_transform.scale.abs();
    let effective = (size * scale).abs().max(Vec3::splat(0.01));

    fn radius_from_effective_size_and_axis(effective: Vec3, axis: Vec3) -> f32 {
        let axis = axis.abs();
        if axis.x >= axis.y && axis.x >= axis.z {
            0.5 * effective.y.max(effective.z).max(0.01)
        } else if axis.y >= axis.x && axis.y >= axis.z {
            0.5 * effective.x.max(effective.z).max(0.01)
        } else {
            0.5 * effective.x.max(effective.y).max(0.01)
        }
    }

    if let Some(spinner) = rig
        .propellers
        .iter()
        .find(|p| p.edge.matches_binding(binding))
    {
        let radius = radius_from_effective_size_and_axis(effective, spinner.spin_axis_local);
        let rad_per_sec = (PROPELLER_RAD_PER_SEC / radius).clamp(-800.0, 800.0);
        return Some(spin_idle_slot(spinner.spin_axis_local, rad_per_sec));
    }

    if let Some(spinner) = rig.rotors.iter().find(|p| p.edge.matches_binding(binding)) {
        let radius = radius_from_effective_size_and_axis(effective, spinner.spin_axis_local);
        let rad_per_sec = (ROTOR_RAD_PER_SEC / radius).clamp(-800.0, 800.0);
        return Some(spin_idle_slot(spinner.spin_axis_local, rad_per_sec));
    }

    None
}

fn biped_kick_override_for_binding(
    rig: &BipedRigV1,
    binding: &ObjectRefEdgeBinding,
    _library: &ObjectLibrary,
    attack_window_secs: f32,
) -> Vec<PartAnimationSlot> {
    const KICK_PITCH_DEG: f32 = 65.0;
    const BODY_ROLL_DEG: f32 = 8.0;

    let duration = attack_window_secs.max(0.05);

    fn hold_slot(duration_secs: f32) -> PartAnimationSlot {
        PartAnimationSlot {
            channel: "attack_primary".into(),
            spec: PartAnimationSpec {
                driver: PartAnimationDriver::AttackTime,
                speed_scale: 1.0,
                time_offset_units: 0.0,
                clip: PartAnimationDef::Once {
                    duration_secs: duration_secs.max(0.05),
                    keyframes: vec![PartAnimationKeyframeDef {
                        time_secs: 0.0,
                        delta: Transform::IDENTITY,
                    }],
                },
            },
        }
    }

    fn kick_slot(duration_secs: f32, pitch_degrees: f32) -> PartAnimationSlot {
        let duration = duration_secs.max(0.05);
        let kick = pitch_degrees.to_radians();
        let wind = duration * 0.18;
        let strike = duration * 0.45;
        let settle = duration * 0.65;

        let keyframes = vec![
            PartAnimationKeyframeDef {
                time_secs: 0.0,
                delta: Transform::IDENTITY,
            },
            PartAnimationKeyframeDef {
                time_secs: wind,
                delta: Transform {
                    rotation: Quat::from_rotation_x(-kick * 0.15),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: strike,
                delta: Transform {
                    rotation: Quat::from_rotation_x(kick),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: settle,
                delta: Transform {
                    rotation: Quat::from_rotation_x(kick * 0.85),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: duration,
                delta: Transform::IDENTITY,
            },
        ];

        PartAnimationSlot {
            channel: "attack_primary".into(),
            spec: PartAnimationSpec {
                driver: PartAnimationDriver::AttackTime,
                speed_scale: 1.0,
                time_offset_units: 0.0,
                clip: PartAnimationDef::Once {
                    duration_secs: duration,
                    keyframes,
                },
            },
        }
    }

    fn body_balance_slot(duration_secs: f32, roll_degrees: f32) -> PartAnimationSlot {
        let duration = duration_secs.max(0.05);
        let roll = roll_degrees.to_radians();
        let wind = duration * 0.18;
        let strike = duration * 0.45;
        let settle = duration * 0.70;

        let keyframes = vec![
            PartAnimationKeyframeDef {
                time_secs: 0.0,
                delta: Transform::IDENTITY,
            },
            PartAnimationKeyframeDef {
                time_secs: wind,
                delta: Transform {
                    rotation: Quat::from_rotation_z(roll * 0.65),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: strike,
                delta: Transform {
                    rotation: Quat::from_rotation_z(roll),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: settle,
                delta: Transform {
                    rotation: Quat::from_rotation_z(roll * 0.80),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: duration,
                delta: Transform::IDENTITY,
            },
        ];

        PartAnimationSlot {
            channel: "attack_primary".into(),
            spec: PartAnimationSpec {
                driver: PartAnimationDriver::AttackTime,
                speed_scale: 1.0,
                time_offset_units: 0.0,
                clip: PartAnimationDef::Once {
                    duration_secs: duration,
                    keyframes,
                },
            },
        }
    }

    // Two deterministic variants per attack:
    // - Variant 0: left-leg kick
    // - Variant 1: right-leg kick
    if rig.left_leg.matches_binding(binding) {
        return vec![kick_slot(duration, KICK_PITCH_DEG), hold_slot(duration)];
    }
    if rig.right_leg.matches_binding(binding) {
        return vec![hold_slot(duration), kick_slot(duration, KICK_PITCH_DEG)];
    }

    if let Some(body) = rig.body.as_ref() {
        if body.matches_binding(binding) {
            // Lean toward the planted leg for balance:
            // - Variant 0 (left kick): lean right (negative roll)
            // - Variant 1 (right kick): lean left (positive roll)
            return vec![
                body_balance_slot(duration, -BODY_ROLL_DEG),
                body_balance_slot(duration, BODY_ROLL_DEG),
            ];
        }
    }

    Vec::new()
}

fn biped_melee_swing_override_for_binding(
    rig: &BipedRigV1,
    binding: &ObjectRefEdgeBinding,
    library: &ObjectLibrary,
    attack_window_secs: f32,
) -> Vec<PartAnimationSlot> {
    fn attachment_matches(
        binding: &ObjectRefEdgeBinding,
        parent_anchor: &str,
        child_anchor: &str,
    ) -> bool {
        binding.attachment.as_ref().is_some_and(|a| {
            a.parent_anchor.as_ref() == parent_anchor && a.child_anchor.as_ref() == child_anchor
        })
    }

    fn child_object_ref_for_attachment(
        library: &ObjectLibrary,
        parent_object_id: u128,
        parent_anchor: &str,
        child_anchor: &str,
    ) -> Option<u128> {
        let def = library.get(parent_object_id)?;
        for part in def.parts.iter() {
            let ObjectPartKind::ObjectRef { object_id } = &part.kind else {
                continue;
            };
            let Some(attachment) = part.attachment.as_ref() else {
                continue;
            };
            if attachment.parent_anchor.as_ref() == parent_anchor
                && attachment.child_anchor.as_ref() == child_anchor
            {
                return Some(*object_id);
            }
        }
        None
    }

    fn q_yxz_deg(yaw_deg: f32, pitch_deg: f32, roll_deg: f32) -> Quat {
        let (yaw, pitch, roll) = (
            yaw_deg.to_radians(),
            pitch_deg.to_radians(),
            roll_deg.to_radians(),
        );
        let q =
            Quat::from_rotation_z(roll) * Quat::from_rotation_x(pitch) * Quat::from_rotation_y(yaw);
        if q.is_finite() {
            q.normalize()
        } else {
            Quat::IDENTITY
        }
    }

    fn attack_slot(
        duration_secs: f32,
        keyframes: Vec<PartAnimationKeyframeDef>,
    ) -> PartAnimationSlot {
        let duration = duration_secs.max(0.05);
        PartAnimationSlot {
            channel: "attack_primary".into(),
            spec: PartAnimationSpec {
                driver: PartAnimationDriver::AttackTime,
                speed_scale: 1.0,
                time_offset_units: 0.0,
                clip: PartAnimationDef::Once {
                    duration_secs: duration,
                    keyframes,
                },
            },
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum BipedSide {
        Left,
        Right,
    }

    fn keyframes_weapon_arm_variants(
        duration: f32,
        side: BipedSide,
    ) -> [Vec<PartAnimationKeyframeDef>; 3] {
        let sign = if side == BipedSide::Left { -1.0 } else { 1.0 };
        let rot = |yaw_deg: f32, pitch_deg: f32, roll_deg: f32| {
            q_yxz_deg(yaw_deg * sign, pitch_deg, roll_deg * sign)
        };
        let t_hold = duration * 0.10;
        let t_wind = duration * 0.28;
        let t_strike = duration * 0.55;

        // Variant 0: outside-top -> inside-bottom (diagonal).
        let v0 = vec![
            PartAnimationKeyframeDef {
                time_secs: 0.0,
                delta: Transform::IDENTITY,
            },
            PartAnimationKeyframeDef {
                time_secs: t_hold,
                delta: Transform {
                    rotation: rot(-10.0, -40.0, -90.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_wind,
                delta: Transform {
                    rotation: rot(-35.0, -65.0, -110.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_strike,
                delta: Transform {
                    rotation: rot(20.0, 25.0, -35.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: duration,
                delta: Transform::IDENTITY,
            },
        ];

        // Variant 1: inside -> outside (horizontal).
        let v1 = vec![
            PartAnimationKeyframeDef {
                time_secs: 0.0,
                delta: Transform::IDENTITY,
            },
            PartAnimationKeyframeDef {
                time_secs: t_hold,
                delta: Transform {
                    rotation: rot(-20.0, -12.0, -90.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_wind,
                delta: Transform {
                    rotation: rot(-35.0, -18.0, -95.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_strike,
                delta: Transform {
                    rotation: rot(35.0, -4.0, -85.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: duration,
                delta: Transform::IDENTITY,
            },
        ];

        // Variant 2: thrust forward.
        let v2 = vec![
            PartAnimationKeyframeDef {
                time_secs: 0.0,
                delta: Transform::IDENTITY,
            },
            PartAnimationKeyframeDef {
                time_secs: t_hold,
                delta: Transform {
                    rotation: rot(0.0, -15.0, -90.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_wind,
                delta: Transform {
                    rotation: rot(0.0, -25.0, -90.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_strike,
                delta: Transform {
                    rotation: rot(0.0, 5.0, -90.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: duration,
                delta: Transform::IDENTITY,
            },
        ];

        [v0, v1, v2]
    }

    fn keyframes_offhand_arm_variants(
        duration: f32,
        side: BipedSide,
    ) -> [Vec<PartAnimationKeyframeDef>; 3] {
        let sign = if side == BipedSide::Right { -1.0 } else { 1.0 };
        let rot = |yaw_deg: f32, pitch_deg: f32, roll_deg: f32| {
            q_yxz_deg(yaw_deg * sign, pitch_deg, roll_deg * sign)
        };
        let t_hold = duration * 0.10;
        let t_wind = duration * 0.28;
        let t_strike = duration * 0.55;

        let v0 = vec![
            PartAnimationKeyframeDef {
                time_secs: 0.0,
                delta: Transform::IDENTITY,
            },
            PartAnimationKeyframeDef {
                time_secs: t_hold,
                delta: Transform {
                    rotation: rot(10.0, -22.0, 20.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_wind,
                delta: Transform {
                    rotation: rot(14.0, -35.0, 30.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_strike,
                delta: Transform {
                    rotation: rot(-5.0, 12.0, -5.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: duration,
                delta: Transform::IDENTITY,
            },
        ];

        let v1 = vec![
            PartAnimationKeyframeDef {
                time_secs: 0.0,
                delta: Transform::IDENTITY,
            },
            PartAnimationKeyframeDef {
                time_secs: t_hold,
                delta: Transform {
                    rotation: rot(-8.0, -10.0, 12.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_wind,
                delta: Transform {
                    rotation: rot(-12.0, -14.0, 15.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_strike,
                delta: Transform {
                    rotation: rot(12.0, -2.0, 8.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: duration,
                delta: Transform::IDENTITY,
            },
        ];

        let v2 = vec![
            PartAnimationKeyframeDef {
                time_secs: 0.0,
                delta: Transform::IDENTITY,
            },
            PartAnimationKeyframeDef {
                time_secs: t_hold,
                delta: Transform {
                    rotation: rot(0.0, -12.0, 8.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_wind,
                delta: Transform {
                    rotation: rot(0.0, -18.0, 10.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_strike,
                delta: Transform {
                    rotation: rot(0.0, 2.0, 6.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: duration,
                delta: Transform::IDENTITY,
            },
        ];

        [v0, v1, v2]
    }

    fn keyframes_body_variants(duration: f32, recoil: f32) -> [Vec<PartAnimationKeyframeDef>; 3] {
        let t_hold = duration * 0.10;
        let t_wind = duration * 0.28;
        let t_strike = duration * 0.55;

        let v0 = vec![
            PartAnimationKeyframeDef {
                time_secs: 0.0,
                delta: Transform::IDENTITY,
            },
            PartAnimationKeyframeDef {
                time_secs: t_hold,
                delta: Transform {
                    rotation: q_yxz_deg(8.0, 0.0, 0.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_wind,
                delta: Transform {
                    translation: -Vec3::Z * (recoil * 0.25),
                    rotation: q_yxz_deg(15.0, -5.0, 0.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_strike,
                delta: Transform {
                    translation: -Vec3::Z * recoil,
                    rotation: q_yxz_deg(-10.0, 6.0, 0.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: duration,
                delta: Transform::IDENTITY,
            },
        ];

        let v1 = vec![
            PartAnimationKeyframeDef {
                time_secs: 0.0,
                delta: Transform::IDENTITY,
            },
            PartAnimationKeyframeDef {
                time_secs: t_hold,
                delta: Transform {
                    rotation: q_yxz_deg(-6.0, 0.0, 0.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_wind,
                delta: Transform {
                    translation: -Vec3::Z * (recoil * 0.20),
                    rotation: q_yxz_deg(-14.0, -3.0, 0.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_strike,
                delta: Transform {
                    translation: -Vec3::Z * recoil,
                    rotation: q_yxz_deg(12.0, 4.0, 0.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: duration,
                delta: Transform::IDENTITY,
            },
        ];

        let v2 = vec![
            PartAnimationKeyframeDef {
                time_secs: 0.0,
                delta: Transform::IDENTITY,
            },
            PartAnimationKeyframeDef {
                time_secs: t_hold,
                delta: Transform {
                    translation: -Vec3::Z * (recoil * 0.10),
                    rotation: q_yxz_deg(0.0, -2.0, 0.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_wind,
                delta: Transform {
                    translation: -Vec3::Z * (recoil * 0.15),
                    rotation: q_yxz_deg(0.0, -6.0, 0.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_strike,
                delta: Transform {
                    translation: Vec3::Z * recoil,
                    rotation: q_yxz_deg(0.0, 8.0, 0.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: duration,
                delta: Transform::IDENTITY,
            },
        ];

        [v0, v1, v2]
    }

    fn keyframes_head_variants(duration: f32) -> [Vec<PartAnimationKeyframeDef>; 3] {
        let t_hold = duration * 0.10;
        let t_wind = duration * 0.28;
        let t_strike = duration * 0.55;

        let v0 = vec![
            PartAnimationKeyframeDef {
                time_secs: 0.0,
                delta: Transform::IDENTITY,
            },
            PartAnimationKeyframeDef {
                time_secs: t_hold,
                delta: Transform {
                    rotation: q_yxz_deg(4.0, -6.0, 0.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_wind,
                delta: Transform {
                    rotation: q_yxz_deg(8.0, -10.0, 0.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_strike,
                delta: Transform {
                    rotation: q_yxz_deg(-6.0, 10.0, 0.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: duration,
                delta: Transform::IDENTITY,
            },
        ];

        let v1 = vec![
            PartAnimationKeyframeDef {
                time_secs: 0.0,
                delta: Transform::IDENTITY,
            },
            PartAnimationKeyframeDef {
                time_secs: t_hold,
                delta: Transform {
                    rotation: q_yxz_deg(-6.0, -4.0, 0.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_wind,
                delta: Transform {
                    rotation: q_yxz_deg(-10.0, -6.0, 0.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_strike,
                delta: Transform {
                    rotation: q_yxz_deg(10.0, 6.0, 0.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: duration,
                delta: Transform::IDENTITY,
            },
        ];

        let v2 = vec![
            PartAnimationKeyframeDef {
                time_secs: 0.0,
                delta: Transform::IDENTITY,
            },
            PartAnimationKeyframeDef {
                time_secs: t_hold,
                delta: Transform {
                    rotation: q_yxz_deg(0.0, -4.0, 0.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_wind,
                delta: Transform {
                    rotation: q_yxz_deg(0.0, -8.0, 0.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_strike,
                delta: Transform {
                    rotation: q_yxz_deg(0.0, 10.0, 0.0),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: duration,
                delta: Transform::IDENTITY,
            },
        ];

        [v0, v1, v2]
    }

    let duration = attack_window_secs.max(0.05);
    let size = library.size(binding.child_object_id).unwrap_or(Vec3::ONE);
    let scale = binding.base_transform.scale.abs();
    let effective = (size * scale).abs().max(Vec3::splat(0.01));
    // Drive additional joints when present in the composition chain (Gen3D convention):
    // shoulder -> upper_arm (rig edge), upper_arm --elbow_mount/joint--> forearm,
    // forearm --wrist_mount/joint--> hand.
    let right_upper = rig.right_arm.as_ref().map(|e| e.child_object_id);
    let left_upper = rig.left_arm.as_ref().map(|e| e.child_object_id);

    let right_lower = right_upper.and_then(|id| {
        child_object_ref_for_attachment(library, id, "elbow_mount", "elbow_joint")
            .or_else(|| child_object_ref_for_attachment(library, id, "elbow_mount", "elbow_mount"))
            .or_else(|| child_object_ref_for_attachment(library, id, "elbow_joint", "elbow_mount"))
            .or_else(|| child_object_ref_for_attachment(library, id, "elbow_joint", "elbow_joint"))
    });
    let left_lower = left_upper.and_then(|id| {
        child_object_ref_for_attachment(library, id, "elbow_mount", "elbow_joint")
            .or_else(|| child_object_ref_for_attachment(library, id, "elbow_mount", "elbow_mount"))
            .or_else(|| child_object_ref_for_attachment(library, id, "elbow_joint", "elbow_mount"))
            .or_else(|| child_object_ref_for_attachment(library, id, "elbow_joint", "elbow_joint"))
    });

    let right_hand = right_lower.and_then(|id| {
        child_object_ref_for_attachment(library, id, "wrist_mount", "wrist_joint")
            .or_else(|| child_object_ref_for_attachment(library, id, "wrist_mount", "wrist_mount"))
            .or_else(|| child_object_ref_for_attachment(library, id, "wrist_joint", "wrist_mount"))
            .or_else(|| child_object_ref_for_attachment(library, id, "wrist_joint", "wrist_joint"))
    });
    let left_hand = left_lower.and_then(|id| {
        child_object_ref_for_attachment(library, id, "wrist_mount", "wrist_joint")
            .or_else(|| child_object_ref_for_attachment(library, id, "wrist_mount", "wrist_mount"))
            .or_else(|| child_object_ref_for_attachment(library, id, "wrist_joint", "wrist_mount"))
            .or_else(|| child_object_ref_for_attachment(library, id, "wrist_joint", "wrist_joint"))
    });

    fn object_has_grip_object_ref(library: &ObjectLibrary, object_id: u128) -> bool {
        let Some(def) = library.get(object_id) else {
            return false;
        };
        def.parts.iter().any(|part| {
            let ObjectPartKind::ObjectRef { .. } = part.kind else {
                return false;
            };
            let Some(att) = part.attachment.as_ref() else {
                return false;
            };
            let parent = att.parent_anchor.as_ref().to_ascii_lowercase();
            let child = att.child_anchor.as_ref().to_ascii_lowercase();
            parent.contains("grip") || child.contains("grip")
        })
    }

    fn object_has_any_object_ref(library: &ObjectLibrary, object_id: u128) -> bool {
        let Some(def) = library.get(object_id) else {
            return false;
        };
        def.parts
            .iter()
            .any(|part| matches!(part.kind, ObjectPartKind::ObjectRef { .. }))
    }

    let left_has_grip = left_hand.is_some_and(|id| object_has_grip_object_ref(library, id));
    let right_has_grip = right_hand.is_some_and(|id| object_has_grip_object_ref(library, id));
    let left_has_any = left_hand.is_some_and(|id| object_has_any_object_ref(library, id));
    let right_has_any = right_hand.is_some_and(|id| object_has_any_object_ref(library, id));

    let weapon_side = match (left_has_grip, right_has_grip) {
        (true, false) => BipedSide::Left,
        (false, true) => BipedSide::Right,
        _ => match (left_has_any, right_has_any) {
            (true, false) => BipedSide::Left,
            (false, true) => BipedSide::Right,
            _ => BipedSide::Right,
        },
    };
    let right_is_weapon_arm = weapon_side == BipedSide::Right;
    let left_is_weapon_arm = weapon_side == BipedSide::Left;

    fn keyframes_elbow_variants(
        duration: f32,
        is_weapon_arm: bool,
    ) -> [Vec<PartAnimationKeyframeDef>; 3] {
        let t_hold = duration * 0.10;
        let t_wind = duration * 0.28;
        let t_strike = duration * 0.55;

        let (hold, wind, strike): (f32, f32, f32) = if is_weapon_arm {
            // Keep a strong bend so the weapon reads as "held" rather than a rigid straight arm.
            (75.0, 95.0, 18.0)
        } else {
            (35.0, 45.0, 10.0)
        };

        // Variant 0: diagonal swing (bend stays fairly constant).
        let v0 = vec![
            PartAnimationKeyframeDef {
                time_secs: 0.0,
                delta: Transform::IDENTITY,
            },
            PartAnimationKeyframeDef {
                time_secs: t_hold,
                delta: Transform {
                    rotation: Quat::from_rotation_x(hold.to_radians()),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_wind,
                delta: Transform {
                    rotation: Quat::from_rotation_x(wind.to_radians()),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_strike,
                delta: Transform {
                    rotation: Quat::from_rotation_x(strike.to_radians()),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: duration,
                delta: Transform::IDENTITY,
            },
        ];

        // Variant 1: horizontal swing (slightly less wind).
        let v1 = vec![
            PartAnimationKeyframeDef {
                time_secs: 0.0,
                delta: Transform::IDENTITY,
            },
            PartAnimationKeyframeDef {
                time_secs: t_hold,
                delta: Transform {
                    rotation: Quat::from_rotation_x((hold * 0.85).to_radians()),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_wind,
                delta: Transform {
                    rotation: Quat::from_rotation_x((wind * 0.80).to_radians()),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_strike,
                delta: Transform {
                    rotation: Quat::from_rotation_x((strike * 1.05).to_radians()),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: duration,
                delta: Transform::IDENTITY,
            },
        ];

        // Variant 2: thrust (bend more in wind, extend on strike).
        let v2 = vec![
            PartAnimationKeyframeDef {
                time_secs: 0.0,
                delta: Transform::IDENTITY,
            },
            PartAnimationKeyframeDef {
                time_secs: t_hold,
                delta: Transform {
                    rotation: Quat::from_rotation_x((hold * 0.95).to_radians()),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_wind,
                delta: Transform {
                    rotation: Quat::from_rotation_x((wind * 1.10).to_radians()),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_strike,
                delta: Transform {
                    rotation: Quat::from_rotation_x((strike * 0.70).to_radians()),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: duration,
                delta: Transform::IDENTITY,
            },
        ];

        [v0, v1, v2]
    }

    fn keyframes_wrist_variants(
        duration: f32,
        is_weapon_arm: bool,
    ) -> [Vec<PartAnimationKeyframeDef>; 3] {
        let t_hold = duration * 0.10;
        let t_wind = duration * 0.28;
        let t_strike = duration * 0.55;

        let (hold_roll, wind_roll, strike_roll): (f32, f32, f32) = if is_weapon_arm {
            (-35.0, -55.0, -10.0)
        } else {
            (10.0, 15.0, 6.0)
        };

        let v0 = vec![
            PartAnimationKeyframeDef {
                time_secs: 0.0,
                delta: Transform::IDENTITY,
            },
            PartAnimationKeyframeDef {
                time_secs: t_hold,
                delta: Transform {
                    rotation: Quat::from_rotation_z(hold_roll.to_radians()),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_wind,
                delta: Transform {
                    rotation: Quat::from_rotation_z(wind_roll.to_radians()),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_strike,
                delta: Transform {
                    rotation: Quat::from_rotation_z(strike_roll.to_radians()),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: duration,
                delta: Transform::IDENTITY,
            },
        ];

        let v1 = vec![
            PartAnimationKeyframeDef {
                time_secs: 0.0,
                delta: Transform::IDENTITY,
            },
            PartAnimationKeyframeDef {
                time_secs: t_hold,
                delta: Transform {
                    rotation: Quat::from_rotation_z((hold_roll * 0.7).to_radians()),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_wind,
                delta: Transform {
                    rotation: Quat::from_rotation_z((wind_roll * 0.6).to_radians()),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_strike,
                delta: Transform {
                    rotation: Quat::from_rotation_z((strike_roll * 0.8).to_radians()),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: duration,
                delta: Transform::IDENTITY,
            },
        ];

        let v2 = vec![
            PartAnimationKeyframeDef {
                time_secs: 0.0,
                delta: Transform::IDENTITY,
            },
            PartAnimationKeyframeDef {
                time_secs: t_hold,
                delta: Transform {
                    rotation: Quat::from_rotation_z((hold_roll * 0.9).to_radians()),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_wind,
                delta: Transform {
                    rotation: Quat::from_rotation_z((wind_roll * 0.9).to_radians()),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: t_strike,
                delta: Transform {
                    rotation: Quat::from_rotation_z((strike_roll * 0.6).to_radians()),
                    ..default()
                },
            },
            PartAnimationKeyframeDef {
                time_secs: duration,
                delta: Transform::IDENTITY,
            },
        ];

        [v0, v1, v2]
    }

    // Elbows.
    if right_upper.is_some_and(|id| binding.parent_object_id == id)
        && right_lower.is_some_and(|id| binding.child_object_id == id)
        && (attachment_matches(binding, "elbow_mount", "elbow_joint")
            || attachment_matches(binding, "elbow_mount", "elbow_mount")
            || attachment_matches(binding, "elbow_joint", "elbow_mount")
            || attachment_matches(binding, "elbow_joint", "elbow_joint"))
    {
        let variants = keyframes_elbow_variants(duration, right_is_weapon_arm);
        return variants
            .into_iter()
            .map(|kfs| attack_slot(duration, kfs))
            .collect();
    }
    if left_upper.is_some_and(|id| binding.parent_object_id == id)
        && left_lower.is_some_and(|id| binding.child_object_id == id)
        && (attachment_matches(binding, "elbow_mount", "elbow_joint")
            || attachment_matches(binding, "elbow_mount", "elbow_mount")
            || attachment_matches(binding, "elbow_joint", "elbow_mount")
            || attachment_matches(binding, "elbow_joint", "elbow_joint"))
    {
        let variants = keyframes_elbow_variants(duration, left_is_weapon_arm);
        return variants
            .into_iter()
            .map(|kfs| attack_slot(duration, kfs))
            .collect();
    }

    // Wrists / hands.
    if right_lower.is_some_and(|id| binding.parent_object_id == id)
        && right_hand.is_some_and(|id| binding.child_object_id == id)
        && (attachment_matches(binding, "wrist_mount", "wrist_joint")
            || attachment_matches(binding, "wrist_mount", "wrist_mount")
            || attachment_matches(binding, "wrist_joint", "wrist_mount")
            || attachment_matches(binding, "wrist_joint", "wrist_joint"))
    {
        let variants = keyframes_wrist_variants(duration, right_is_weapon_arm);
        return variants
            .into_iter()
            .map(|kfs| attack_slot(duration, kfs))
            .collect();
    }
    if left_lower.is_some_and(|id| binding.parent_object_id == id)
        && left_hand.is_some_and(|id| binding.child_object_id == id)
        && (attachment_matches(binding, "wrist_mount", "wrist_joint")
            || attachment_matches(binding, "wrist_mount", "wrist_mount")
            || attachment_matches(binding, "wrist_joint", "wrist_mount")
            || attachment_matches(binding, "wrist_joint", "wrist_joint"))
    {
        let variants = keyframes_wrist_variants(duration, left_is_weapon_arm);
        return variants
            .into_iter()
            .map(|kfs| attack_slot(duration, kfs))
            .collect();
    }

    if let Some(right_arm) = rig.right_arm.as_ref() {
        if right_arm.matches_binding(binding) {
            let variants = if right_is_weapon_arm {
                keyframes_weapon_arm_variants(duration, BipedSide::Right)
            } else {
                keyframes_offhand_arm_variants(duration, BipedSide::Right)
            };
            return variants
                .into_iter()
                .map(|kfs| attack_slot(duration, kfs))
                .collect();
        }
    }
    if let Some(left_arm) = rig.left_arm.as_ref() {
        if left_arm.matches_binding(binding) {
            let variants = if left_is_weapon_arm {
                keyframes_weapon_arm_variants(duration, BipedSide::Left)
            } else {
                keyframes_offhand_arm_variants(duration, BipedSide::Left)
            };
            return variants
                .into_iter()
                .map(|kfs| attack_slot(duration, kfs))
                .collect();
        }
    }

    if let Some(body) = rig.body.as_ref() {
        if body.matches_binding(binding) {
            let recoil = (effective.z * 0.02).clamp(0.0025, 0.08);
            let variants = keyframes_body_variants(duration, recoil);
            return variants
                .into_iter()
                .map(|kfs| attack_slot(duration, kfs))
                .collect();
        }
    }

    if let Some(head) = rig.head.as_ref() {
        if head.matches_binding(binding) {
            let variants = keyframes_head_variants(duration);
            return variants
                .into_iter()
                .map(|kfs| attack_slot(duration, kfs))
                .collect();
        }
    }

    Vec::new()
}

fn quadruped_bite_override_for_binding(
    rig: &QuadrupedRigV1,
    binding: &ObjectRefEdgeBinding,
    library: &ObjectLibrary,
    attack_window_secs: f32,
) -> Option<PartAnimationSlot> {
    if let Some(head) = rig.head.as_ref() {
        if head.matches_binding(binding) {
            let size = library.size(binding.child_object_id).unwrap_or(Vec3::ONE);
            let scale = binding.base_transform.scale.abs();
            let effective = (size * scale).abs().max(Vec3::splat(0.01));
            let lunge = (effective.z * 0.22).clamp(0.01, 0.40);
            return Some(bite_attack_slot(attack_window_secs, lunge, 25.0));
        }
    }

    if let Some(body) = rig.body.as_ref() {
        if body.matches_binding(binding) {
            let size = library.size(binding.child_object_id).unwrap_or(Vec3::ONE);
            let scale = binding.base_transform.scale.abs();
            let effective = (size * scale).abs().max(Vec3::splat(0.01));
            let recoil = (effective.z * 0.015).clamp(0.0025, 0.06);
            return Some(recoil_attack_slot(-Vec3::Z, attack_window_secs, recoil));
        }
    }

    None
}

fn tool_arm_dig_override_for_binding(
    rig: &CarRigV1,
    binding: &ObjectRefEdgeBinding,
    _library: &ObjectLibrary,
    attack_window_secs: f32,
) -> Option<PartAnimationSlot> {
    const AMP_PROX_DEG: f32 = 18.0;
    const AMP_DIST_DEG: f32 = 55.0;

    for tool_arm in &rig.tool_arms {
        for (idx, joint) in tool_arm.joints.iter().enumerate() {
            if !joint.matches_binding(binding) {
                continue;
            }

            let count = tool_arm.joints.len().max(1);
            let t = if count > 1 {
                (idx as f32) / ((count - 1) as f32)
            } else {
                0.0
            };
            let amp = AMP_PROX_DEG + (AMP_DIST_DEG - AMP_PROX_DEG) * t.clamp(0.0, 1.0);
            // Generic "dig" motion: push the arm chain forward/down, then curl the end effector.
            let sign = if idx + 1 == count { -1.0 } else { 1.0 };
            return Some(swing_attack_slot(Vec3::X, attack_window_secs, amp * sign));
        }
    }

    None
}

fn ranged_recoil_override_for_binding(
    rig: &MotionRigV1,
    binding: &ObjectRefEdgeBinding,
    library: &ObjectLibrary,
    attack_window_secs: f32,
) -> Option<PartAnimationSlot> {
    match rig {
        MotionRigV1::Biped(rig) => {
            if let Some(body) = rig.body.as_ref() {
                if body.matches_binding(binding) {
                    let size = library.size(binding.child_object_id).unwrap_or(Vec3::ONE);
                    let scale = binding.base_transform.scale.abs();
                    let effective = (size * scale).abs().max(Vec3::splat(0.01));
                    let recoil = (effective.z * 0.015).clamp(0.0025, 0.06);
                    return Some(recoil_attack_slot(-Vec3::Z, attack_window_secs, recoil));
                }
            }
            if let Some(right_arm) = rig.right_arm.as_ref() {
                if right_arm.matches_binding(binding) {
                    return Some(recoil_attack_slot(-Vec3::Z, attack_window_secs, 0.02));
                }
            }
            if let Some(left_arm) = rig.left_arm.as_ref() {
                if left_arm.matches_binding(binding) {
                    return Some(recoil_attack_slot(-Vec3::Z, attack_window_secs, 0.02));
                }
            }
            if let Some(head) = rig.head.as_ref() {
                if head.matches_binding(binding) {
                    return Some(swing_attack_slot(Vec3::X, attack_window_secs, 10.0));
                }
            }
            None
        }
        MotionRigV1::Quadruped(rig) => {
            if let Some(body) = rig.body.as_ref() {
                if body.matches_binding(binding) {
                    let size = library.size(binding.child_object_id).unwrap_or(Vec3::ONE);
                    let scale = binding.base_transform.scale.abs();
                    let effective = (size * scale).abs().max(Vec3::splat(0.01));
                    let recoil = (effective.z * 0.012).clamp(0.0025, 0.05);
                    return Some(recoil_attack_slot(-Vec3::Z, attack_window_secs, recoil));
                }
            }
            if let Some(head) = rig.head.as_ref() {
                if head.matches_binding(binding) {
                    return Some(swing_attack_slot(Vec3::X, attack_window_secs, 12.0));
                }
            }
            None
        }
        MotionRigV1::Car(rig) => {
            if let Some(body) = rig.body.as_ref() {
                if body.matches_binding(binding) {
                    let size = library.size(binding.child_object_id).unwrap_or(Vec3::ONE);
                    let scale = binding.base_transform.scale.abs();
                    let effective = (size * scale).abs().max(Vec3::splat(0.01));
                    let recoil = (effective.z * 0.01).clamp(0.0025, 0.05);
                    return Some(recoil_attack_slot(-Vec3::Z, attack_window_secs, recoil));
                }
            }
            None
        }
        MotionRigV1::Airplane(rig) => {
            if let Some(body) = rig.body.as_ref() {
                if body.matches_binding(binding) {
                    let size = library.size(binding.child_object_id).unwrap_or(Vec3::ONE);
                    let scale = binding.base_transform.scale.abs();
                    let effective = (size * scale).abs().max(Vec3::splat(0.01));
                    let recoil = (effective.z * 0.01).clamp(0.0025, 0.05);
                    return Some(recoil_attack_slot(-Vec3::Z, attack_window_secs, recoil));
                }
            }
            None
        }
    }
}

fn anchor_transform(library: &ObjectLibrary, object_id: u128, name: &str) -> Option<Transform> {
    if name == "origin" {
        return Some(Transform::IDENTITY);
    }
    library
        .get(object_id)
        .and_then(|def| def.anchors.iter().find(|a| a.name.as_ref() == name))
        .map(|a| a.transform)
}

fn binding_base_pose_transform(
    binding: &ObjectRefEdgeBinding,
    library: &ObjectLibrary,
) -> Transform {
    let Some(attachment) = binding.attachment.as_ref() else {
        return binding.base_transform;
    };

    let parent_anchor = anchor_transform(
        library,
        binding.parent_object_id,
        attachment.parent_anchor.as_ref(),
    )
    .unwrap_or(Transform::IDENTITY);
    let child_anchor = anchor_transform(
        library,
        binding.child_object_id,
        attachment.child_anchor.as_ref(),
    )
    .unwrap_or(Transform::IDENTITY);
    let composed = parent_anchor.to_matrix()
        * binding.base_transform.to_matrix()
        * child_anchor.to_matrix().inverse();
    crate::geometry::mat4_to_transform_allow_degenerate_scale(composed)
        .unwrap_or(binding.base_transform)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::registry::{
        AnchorDef, AttachmentDef, ColliderProfile, ObjectDef, ObjectInteraction, ObjectPartDef,
        ObjectPartKind, PrimitiveVisualDef,
    };
    use crate::prefab_descriptors::{
        PrefabDescriptorFileV1, PrefabDescriptorInterfacesV1, PrefabDescriptorLibrary,
    };
    use serde_json::json;

    fn make_descriptor(prefab_id: u128, rig: serde_json::Value) -> PrefabDescriptorFileV1 {
        let mut extra: std::collections::BTreeMap<String, serde_json::Value> = Default::default();
        extra.insert("motion_rig_v1".to_string(), rig);
        PrefabDescriptorFileV1 {
            format_version: crate::prefab_descriptors::PREFAB_DESCRIPTOR_FORMAT_VERSION,
            prefab_id: uuid::Uuid::from_u128(prefab_id).to_string(),
            label: None,
            text: None,
            tags: Vec::new(),
            roles: Vec::new(),
            interfaces: Some(PrefabDescriptorInterfacesV1 {
                anchors: Vec::new(),
                animation_channels: Vec::new(),
                notes: None,
                extra,
            }),
            provenance: None,
            extra: Default::default(),
        }
    }

    fn make_library_with_anchors(
        parent_id: u128,
        parent_anchor: &str,
        child_id: u128,
        child_anchor: &str,
    ) -> ObjectLibrary {
        let mut library = ObjectLibrary::default();
        library.upsert(ObjectDef {
            object_id: parent_id,
            label: "parent".into(),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: vec![AnchorDef {
                name: parent_anchor.to_string().into(),
                transform: Transform::IDENTITY,
            }],
            parts: vec![ObjectPartDef {
                part_id: None,
                render_priority: None,
                kind: ObjectPartKind::Primitive {
                    primitive: PrimitiveVisualDef::Primitive {
                        mesh: crate::object::registry::MeshKey::UnitCube,
                        params: None,
                        color: Color::WHITE,
                        unlit: false,
                    },
                },
                attachment: None,
                animations: Vec::new(),
                transform: Transform::IDENTITY,
            }],
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        });
        library.upsert(ObjectDef {
            object_id: child_id,
            label: "child".into(),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: vec![AnchorDef {
                name: child_anchor.to_string().into(),
                transform: Transform::IDENTITY,
            }],
            parts: Vec::new(),
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        });
        library
    }

    #[test]
    fn motion_rig_v1_parses_biped() {
        let prefab_id = 0xA11CE_u128;
        let leg_l = 0x10_u128;
        let leg_r = 0x11_u128;

        let rig = json!({
            "version": 1,
            "kind": "biped_v1",
            "move_cycle_m": 1.2,
            "walk_swing_degrees": 30.0,
            "biped": {
                "left_leg": {
                    "parent_object_id": uuid::Uuid::from_u128(prefab_id).to_string(),
                    "child_object_id": uuid::Uuid::from_u128(leg_l).to_string(),
                    "parent_anchor": "hip_l",
                    "child_anchor": "root",
                },
                "right_leg": {
                    "parent_object_id": uuid::Uuid::from_u128(prefab_id).to_string(),
                    "child_object_id": uuid::Uuid::from_u128(leg_r).to_string(),
                    "parent_anchor": "hip_r",
                    "child_anchor": "root",
                },
            }
        });

        let doc = make_descriptor(prefab_id, rig);
        let parsed = motion_rig_v1_from_descriptor(&doc)
            .expect("parse ok")
            .expect("rig present");

        match parsed {
            MotionRigV1::Biped(rig) => {
                assert!((rig.move_cycle_m - 1.2).abs() < 1e-6);
                assert!((rig.walk_swing_degrees - 30.0).abs() < 1e-6);
                assert_eq!(rig.left_leg.parent_object_id, prefab_id);
                assert_eq!(rig.left_leg.child_object_id, leg_l);
                assert_eq!(rig.left_leg.parent_anchor, "hip_l".to_string());
                assert_eq!(rig.left_leg.child_anchor, "root".to_string());
            }
            other => panic!("expected biped rig, got {other:?}"),
        }
    }

    #[test]
    fn motion_rig_v1_normalizes_left_right_from_anchor_names() {
        let prefab_id = 0xB1E1D_u128;
        let leg_l = 0x10_u128;
        let leg_r = 0x11_u128;
        let arm_l = 0x20_u128;
        let arm_r = 0x21_u128;

        let rig = json!({
            "version": 1,
            "kind": "biped_v1",
            "biped": {
                // Intentionally swapped: left points at *_R_* and right points at *_L_*.
                "left_leg": {
                    "parent_object_id": uuid::Uuid::from_u128(prefab_id).to_string(),
                    "child_object_id": uuid::Uuid::from_u128(leg_l).to_string(),
                    "parent_anchor": "hip_R_joint",
                    "child_anchor": "root",
                },
                "right_leg": {
                    "parent_object_id": uuid::Uuid::from_u128(prefab_id).to_string(),
                    "child_object_id": uuid::Uuid::from_u128(leg_r).to_string(),
                    "parent_anchor": "hip_L_joint",
                    "child_anchor": "root",
                },
                "left_arm": {
                    "parent_object_id": uuid::Uuid::from_u128(prefab_id).to_string(),
                    "child_object_id": uuid::Uuid::from_u128(arm_l).to_string(),
                    "parent_anchor": "shoulder_R_joint",
                    "child_anchor": "root",
                },
                "right_arm": {
                    "parent_object_id": uuid::Uuid::from_u128(prefab_id).to_string(),
                    "child_object_id": uuid::Uuid::from_u128(arm_r).to_string(),
                    "parent_anchor": "shoulder_L_joint",
                    "child_anchor": "root",
                },
            }
        });

        let doc = make_descriptor(prefab_id, rig);
        let parsed = motion_rig_v1_from_descriptor(&doc)
            .expect("parse ok")
            .expect("rig present");

        let MotionRigV1::Biped(parsed) = parsed else {
            panic!("expected biped rig");
        };

        assert_eq!(parsed.left_leg.parent_anchor, "hip_L_joint");
        assert_eq!(parsed.right_leg.parent_anchor, "hip_R_joint");
        assert_eq!(parsed.left_arm.unwrap().parent_anchor, "shoulder_L_joint");
        assert_eq!(parsed.right_arm.unwrap().parent_anchor, "shoulder_R_joint");
    }

    #[test]
    fn motion_rig_v1_parses_car_with_tool_arm() {
        let prefab_id = 0xCA7_u128;
        let wheel = 0x10_u128;
        let boom = 0x20_u128;
        let bucket = 0x21_u128;

        let rig = json!({
            "version": 1,
            "kind": "car_v1",
            "car": {
                "wheels": [{
                    "edge": {
                        "parent_object_id": uuid::Uuid::from_u128(prefab_id).to_string(),
                        "child_object_id": uuid::Uuid::from_u128(wheel).to_string(),
                        "parent_anchor": "axle",
                        "child_anchor": "root",
                    }
                }],
                "tool_arms": [{
                    "joints": [
                        {
                            "parent_object_id": uuid::Uuid::from_u128(prefab_id).to_string(),
                            "child_object_id": uuid::Uuid::from_u128(boom).to_string(),
                            "parent_anchor": "boom_mount",
                            "child_anchor": "root",
                        },
                        {
                            "parent_object_id": uuid::Uuid::from_u128(boom).to_string(),
                            "child_object_id": uuid::Uuid::from_u128(bucket).to_string(),
                            "parent_anchor": "bucket_mount",
                            "child_anchor": "root",
                        }
                    ]
                }],
            }
        });

        let doc = make_descriptor(prefab_id, rig);
        let parsed = motion_rig_v1_from_descriptor(&doc)
            .expect("parse ok")
            .expect("rig present");

        let MotionRigV1::Car(parsed) = parsed else {
            panic!("expected car rig");
        };
        assert_eq!(parsed.wheels.len(), 1);
        assert_eq!(parsed.tool_arms.len(), 1);
        assert_eq!(parsed.tool_arms[0].joints.len(), 2);
        assert_eq!(parsed.tool_arms[0].joints[0].child_object_id, boom);
        assert_eq!(parsed.tool_arms[0].joints[1].child_object_id, bucket);

        let parsed = MotionRigV1::Car(parsed);
        let applicable = parsed.applicable_attack_primary_algorithms(None);
        assert!(applicable.contains(&AttackPrimaryMotionAlgorithm::ToolArmDigV1));
        let applicable = parsed.applicable_attack_primary_algorithms(Some(UnitAttackKind::Melee));
        assert!(applicable.contains(&AttackPrimaryMotionAlgorithm::ToolArmDigV1));
    }

    #[test]
    fn melee_swing_picks_weapon_arm_from_hand_grip_attachment() {
        let upper_l = 0x200_u128;
        let lower_l = 0x201_u128;
        let hand_l = 0x202_u128;
        let upper_r = 0x210_u128;
        let lower_r = 0x211_u128;
        let hand_r = 0x212_u128;
        let weapon = 0x300_u128;

        fn stub_def(object_id: u128, label: &str, parts: Vec<ObjectPartDef>) -> ObjectDef {
            ObjectDef {
                object_id,
                label: label.to_string().into(),
                size: Vec3::ONE,
                ground_origin_y: None,
                collider: ColliderProfile::None,
                interaction: ObjectInteraction::none(),
                aim: None,
                mobility: None,
                anchors: Vec::new(),
                parts,
                minimap_color: None,
                health_bar_offset_y: None,
                enemy: None,
                muzzle: None,
                projectile: None,
                attack: None,
            }
        }

        let mut library = ObjectLibrary::default();
        library.upsert(stub_def(
            upper_l,
            "upper_l",
            vec![
                ObjectPartDef::object_ref(lower_l, Transform::IDENTITY).with_attachment(
                    AttachmentDef {
                        parent_anchor: "elbow_mount".into(),
                        child_anchor: "elbow_joint".into(),
                    },
                ),
            ],
        ));
        library.upsert(stub_def(
            lower_l,
            "lower_l",
            vec![
                ObjectPartDef::object_ref(hand_l, Transform::IDENTITY).with_attachment(
                    AttachmentDef {
                        parent_anchor: "wrist_mount".into(),
                        child_anchor: "wrist_joint".into(),
                    },
                ),
            ],
        ));
        library.upsert(stub_def(
            hand_l,
            "hand_l",
            vec![
                ObjectPartDef::object_ref(weapon, Transform::IDENTITY).with_attachment(
                    AttachmentDef {
                        parent_anchor: "hand_grip".into(),
                        child_anchor: "grip_joint".into(),
                    },
                ),
            ],
        ));

        library.upsert(stub_def(
            upper_r,
            "upper_r",
            vec![
                ObjectPartDef::object_ref(lower_r, Transform::IDENTITY).with_attachment(
                    AttachmentDef {
                        parent_anchor: "elbow_mount".into(),
                        child_anchor: "elbow_joint".into(),
                    },
                ),
            ],
        ));
        library.upsert(stub_def(
            lower_r,
            "lower_r",
            vec![
                ObjectPartDef::object_ref(hand_r, Transform::IDENTITY).with_attachment(
                    AttachmentDef {
                        parent_anchor: "wrist_mount".into(),
                        child_anchor: "wrist_joint".into(),
                    },
                ),
            ],
        ));
        library.upsert(stub_def(hand_r, "hand_r", Vec::new()));
        library.upsert(stub_def(weapon, "weapon", Vec::new()));

        let mut rig = BipedRigV1 {
            move_cycle_m: 1.0,
            walk_swing_degrees: 25.0,
            default_move_algorithm: None,
            body: None,
            left_leg: MotionEdgeRefV1 {
                parent_object_id: 0x1,
                child_object_id: 0x2,
                parent_anchor: "hip_l".into(),
                child_anchor: "root".into(),
            },
            right_leg: MotionEdgeRefV1 {
                parent_object_id: 0x1,
                child_object_id: 0x3,
                parent_anchor: "hip_r".into(),
                child_anchor: "root".into(),
            },
            left_arm: Some(MotionEdgeRefV1 {
                parent_object_id: 0x10,
                child_object_id: upper_l,
                parent_anchor: "shoulder_mount_L".into(),
                child_anchor: "shoulder_joint".into(),
            }),
            right_arm: Some(MotionEdgeRefV1 {
                parent_object_id: 0x10,
                child_object_id: upper_r,
                parent_anchor: "shoulder_mount_R".into(),
                child_anchor: "shoulder_joint".into(),
            }),
            head: None,
            tail: None,
            ears: Vec::new(),
        };
        rig.normalize_left_right_hints();

        let root = Entity::from_bits(1);
        let left_elbow_binding = ObjectRefEdgeBinding {
            root_entity: root,
            parent_object_id: upper_l,
            child_object_id: lower_l,
            attachment: Some(AttachmentDef {
                parent_anchor: "elbow_mount".into(),
                child_anchor: "elbow_joint".into(),
            }),
            base_transform: Transform::IDENTITY,
            base_slots: Vec::new(),
            apply_aim_yaw: false,
        };
        let right_elbow_binding = ObjectRefEdgeBinding {
            root_entity: root,
            parent_object_id: upper_r,
            child_object_id: lower_r,
            attachment: Some(AttachmentDef {
                parent_anchor: "elbow_mount".into(),
                child_anchor: "elbow_joint".into(),
            }),
            base_transform: Transform::IDENTITY,
            base_slots: Vec::new(),
            apply_aim_yaw: false,
        };

        let left_slots =
            biped_melee_swing_override_for_binding(&rig, &left_elbow_binding, &library, 1.0);
        assert_eq!(left_slots.len(), 3);
        let PartAnimationDef::Once { keyframes, .. } = &left_slots[0].spec.clip else {
            panic!("expected once clip");
        };
        let expected = Quat::from_rotation_x(75.0_f32.to_radians());
        assert!(
            keyframes[1].delta.rotation.angle_between(expected) < 1e-5,
            "expected left elbow to be treated as weapon arm"
        );

        let right_slots =
            biped_melee_swing_override_for_binding(&rig, &right_elbow_binding, &library, 1.0);
        assert_eq!(right_slots.len(), 3);
        let PartAnimationDef::Once { keyframes, .. } = &right_slots[0].spec.clip else {
            panic!("expected once clip");
        };
        let expected = Quat::from_rotation_x(35.0_f32.to_radians());
        assert!(
            keyframes[1].delta.rotation.angle_between(expected) < 1e-5,
            "expected right elbow to be treated as offhand"
        );
    }

    #[test]
    fn biped_kick_provides_left_and_right_variants() {
        use crate::object::visuals::ObjectRefEdgeBinding;

        let mut rig = BipedRigV1 {
            move_cycle_m: 1.0,
            walk_swing_degrees: 25.0,
            default_move_algorithm: None,
            body: None,
            left_leg: MotionEdgeRefV1 {
                parent_object_id: 0x1,
                child_object_id: 0x2,
                parent_anchor: "hip_l".into(),
                child_anchor: "root".into(),
            },
            right_leg: MotionEdgeRefV1 {
                parent_object_id: 0x1,
                child_object_id: 0x3,
                parent_anchor: "hip_r".into(),
                child_anchor: "root".into(),
            },
            left_arm: None,
            right_arm: None,
            head: None,
            tail: None,
            ears: Vec::new(),
        };
        rig.normalize_left_right_hints();

        let library = ObjectLibrary::default();
        let root = Entity::from_bits(1);

        let left_binding = ObjectRefEdgeBinding {
            root_entity: root,
            parent_object_id: 0x1,
            child_object_id: 0x2,
            attachment: Some(AttachmentDef {
                parent_anchor: "hip_l".into(),
                child_anchor: "root".into(),
            }),
            base_transform: Transform::IDENTITY,
            base_slots: Vec::new(),
            apply_aim_yaw: false,
        };
        let right_binding = ObjectRefEdgeBinding {
            root_entity: root,
            parent_object_id: 0x1,
            child_object_id: 0x3,
            attachment: Some(AttachmentDef {
                parent_anchor: "hip_r".into(),
                child_anchor: "root".into(),
            }),
            base_transform: Transform::IDENTITY,
            base_slots: Vec::new(),
            apply_aim_yaw: false,
        };

        let left_slots = biped_kick_override_for_binding(&rig, &left_binding, &library, 1.0);
        assert_eq!(left_slots.len(), 2);
        let PartAnimationDef::Once { keyframes, .. } = &left_slots[0].spec.clip else {
            panic!("expected once clip");
        };
        let expected = Quat::from_rotation_x(65.0_f32.to_radians());
        assert!(
            keyframes[2].delta.rotation.angle_between(expected) < 1e-5,
            "expected left leg variant 0 to kick"
        );
        let PartAnimationDef::Once { keyframes, .. } = &left_slots[1].spec.clip else {
            panic!("expected once clip");
        };
        assert!(
            keyframes[0].delta.rotation.angle_between(Quat::IDENTITY) < 1e-6,
            "expected left leg variant 1 to hold"
        );

        let right_slots = biped_kick_override_for_binding(&rig, &right_binding, &library, 1.0);
        assert_eq!(right_slots.len(), 2);
        let PartAnimationDef::Once { keyframes, .. } = &right_slots[0].spec.clip else {
            panic!("expected once clip");
        };
        assert!(
            keyframes[0].delta.rotation.angle_between(Quat::IDENTITY) < 1e-6,
            "expected right leg variant 0 to hold"
        );
        let PartAnimationDef::Once { keyframes, .. } = &right_slots[1].spec.clip else {
            panic!("expected once clip");
        };
        assert!(
            keyframes[2].delta.rotation.angle_between(expected) < 1e-5,
            "expected right leg variant 1 to kick"
        );
    }

    #[test]
    fn walk_swing_slot_is_move_phase_loop() {
        let slot = walk_swing_move_slot(2.0, 20.0, 0.3);
        assert_eq!(slot.channel.as_ref(), "move");
        assert_eq!(slot.spec.driver, PartAnimationDriver::MovePhase);
        assert!((slot.spec.speed_scale - 1.0).abs() < 1e-6);
        assert!((slot.spec.time_offset_units - 0.3).abs() < 1e-6);
        match &slot.spec.clip {
            PartAnimationDef::Loop {
                duration_secs,
                keyframes,
            } => {
                assert!((*duration_secs - 2.0).abs() < 1e-6);
                assert_eq!(keyframes.len(), 4);
                assert!((keyframes[1].time_secs - 0.5).abs() < 1e-6);
                assert!((keyframes[2].time_secs - 1.0).abs() < 1e-6);
                assert!((keyframes[3].time_secs - 1.5).abs() < 1e-6);
            }
            other => panic!("expected loop clip, got {other:?}"),
        }
    }

    #[test]
    fn car_wheels_spin_uses_ambient_channel_and_flips_under_mirror_scale() {
        use crate::object::visuals::ObjectRefEdgeBinding;

        let parent_id = 0xCA7_u128;
        let wheel_id = 0x10_u128;
        let mut library = make_library_with_anchors(parent_id, "axle", wheel_id, "root");
        let mut wheel_def = library.get(wheel_id).expect("wheel").clone();
        wheel_def.size = Vec3::new(0.2, 1.0, 1.0);
        library.upsert(wheel_def);

        let edge = MotionEdgeRefV1 {
            parent_object_id: parent_id,
            child_object_id: wheel_id,
            parent_anchor: "axle".into(),
            child_anchor: "root".into(),
        };

        let rig = CarRigV1 {
            default_move_algorithm: None,
            body: None,
            wheels: vec![SpinEffectorV1 {
                edge: edge.clone(),
                spin_axis_local: Vec3::Z,
            }],
            tool_arms: Vec::new(),
            wheel_radius_m: None,
            radians_per_meter: Some(1.0),
        };

        let mut world = World::new();
        let root = world.spawn_empty().id();
        let binding = ObjectRefEdgeBinding {
            root_entity: root,
            parent_object_id: parent_id,
            child_object_id: wheel_id,
            attachment: Some(AttachmentDef {
                parent_anchor: "axle".into(),
                child_anchor: "root".into(),
            }),
            base_transform: Transform::IDENTITY,
            base_slots: Vec::new(),
            apply_aim_yaw: false,
        };

        let slot = car_wheels_override_for_binding(&rig, &binding, &library).expect("slot");
        assert_eq!(slot.channel.as_ref(), "ambient");
        assert_eq!(slot.spec.driver, PartAnimationDriver::MoveDistance);
        let PartAnimationDef::Spin {
            axis,
            radians_per_unit,
        } = slot.spec.clip
        else {
            panic!("expected spin clip");
        };
        assert_eq!(axis, Vec3::X);
        assert!((radians_per_unit - 1.0).abs() < 1e-6);

        let mirrored = ObjectRefEdgeBinding {
            base_transform: Transform::from_scale(Vec3::new(-1.0, 1.0, 1.0)),
            ..binding
        };
        let slot = car_wheels_override_for_binding(&rig, &mirrored, &library).expect("slot");
        let PartAnimationDef::Spin {
            axis,
            radians_per_unit,
        } = slot.spec.clip
        else {
            panic!("expected spin clip");
        };
        assert_eq!(axis, Vec3::X);
        assert!((radians_per_unit + 1.0).abs() < 1e-6);
    }

    #[test]
    fn car_wheels_spin_axis_uses_smallest_extent_axis() {
        use crate::object::visuals::ObjectRefEdgeBinding;

        let parent_id = 0xCA7_u128;
        let wheel_id = 0x11_u128;
        let mut library = make_library_with_anchors(parent_id, "axle", wheel_id, "root");
        let mut wheel_def = library.get(wheel_id).expect("wheel").clone();
        wheel_def.size = Vec3::new(1.0, 1.0, 0.2);
        library.upsert(wheel_def);

        let edge = MotionEdgeRefV1 {
            parent_object_id: parent_id,
            child_object_id: wheel_id,
            parent_anchor: "axle".into(),
            child_anchor: "root".into(),
        };

        let rig = CarRigV1 {
            default_move_algorithm: None,
            body: None,
            wheels: vec![SpinEffectorV1 {
                edge: edge.clone(),
                spin_axis_local: Vec3::X,
            }],
            tool_arms: Vec::new(),
            wheel_radius_m: None,
            radians_per_meter: Some(1.0),
        };

        let mut world = World::new();
        let root = world.spawn_empty().id();
        let binding = ObjectRefEdgeBinding {
            root_entity: root,
            parent_object_id: parent_id,
            child_object_id: wheel_id,
            attachment: Some(AttachmentDef {
                parent_anchor: "axle".into(),
                child_anchor: "root".into(),
            }),
            base_transform: Transform::IDENTITY,
            base_slots: Vec::new(),
            apply_aim_yaw: false,
        };

        let slot = car_wheels_override_for_binding(&rig, &binding, &library).expect("slot");
        let PartAnimationDef::Spin { axis, .. } = slot.spec.clip else {
            panic!("expected spin clip");
        };
        assert_eq!(axis, Vec3::Z);
    }

    #[test]
    fn controller_none_removes_player_and_restores_transform() {
        let prefab_id = 0xABCD_u128;
        let child_id = 0xBCDE_u128;
        let library = make_library_with_anchors(prefab_id, "origin", child_id, "origin");
        let descriptors = PrefabDescriptorLibrary::default();

        let mut app = App::new();
        app.insert_resource(library);
        app.insert_resource(descriptors);
        app.add_systems(Update, apply_motion_algorithms_on_controller_change);

        let root = app.world_mut().spawn((ObjectPrefabId(prefab_id),)).id();

        let base = Transform::from_translation(Vec3::new(1.0, 2.0, 3.0));
        let rotated = Transform {
            rotation: Quat::from_rotation_y(core::f32::consts::FRAC_PI_2),
            ..base
        };

        let edge = app
            .world_mut()
            .spawn((
                rotated,
                ObjectRefEdgeBinding {
                    root_entity: root,
                    parent_object_id: prefab_id,
                    child_object_id: child_id,
                    attachment: Some(AttachmentDef {
                        parent_anchor: "origin".into(),
                        child_anchor: "origin".into(),
                    }),
                    base_transform: base,
                    base_slots: Vec::new(),
                    apply_aim_yaw: false,
                },
                PartAnimationPlayer {
                    root_entity: root,
                    parent_object_id: prefab_id,
                    child_object_id: Some(child_id),
                    attachment: Some(AttachmentDef {
                        parent_anchor: "origin".into(),
                        child_anchor: "origin".into(),
                    }),
                    base_transform: base,
                    animations: vec![walk_swing_move_slot(1.0, 10.0, 0.0)],
                    apply_aim_yaw: false,
                },
            ))
            .id();

        // Clear initial change ticks.
        app.update();

        // Now apply a controller state that removes motion overrides.
        app.world_mut()
            .entity_mut(root)
            .insert(MotionAlgorithmController::default());
        app.update();

        assert!(app.world().get::<PartAnimationPlayer>(edge).is_none());
        let t = app.world().get::<Transform>(edge).expect("transform");
        assert!(
            (t.translation - base.translation).length() < 1e-5,
            "expected translation restored to base: base={:?} got={:?}",
            base.translation,
            t.translation
        );
        assert!(
            t.rotation.angle_between(base.rotation) < 1e-5,
            "expected rotation restored to base"
        );
    }

    #[test]
    fn controller_biped_inserts_player_for_matching_edge() {
        let prefab_id = 0xDEAD_u128;
        let leg_l = 0x10_u128;
        let leg_r = 0x11_u128;

        let mut descriptors = PrefabDescriptorLibrary::default();
        descriptors.upsert(
            prefab_id,
            make_descriptor(
                prefab_id,
                json!({
                    "version": 1,
                    "kind": "biped_v1",
                    "move_cycle_m": 1.0,
                    "walk_swing_degrees": 25.0,
                    "biped": {
                        "left_leg": {
                            "parent_object_id": uuid::Uuid::from_u128(prefab_id).to_string(),
                            "child_object_id": uuid::Uuid::from_u128(leg_l).to_string(),
                            "parent_anchor": "hip_l",
                            "child_anchor": "root",
                        },
                        "right_leg": {
                            "parent_object_id": uuid::Uuid::from_u128(prefab_id).to_string(),
                            "child_object_id": uuid::Uuid::from_u128(leg_r).to_string(),
                            "parent_anchor": "hip_r",
                            "child_anchor": "root",
                        },
                    }
                }),
            ),
        );

        let library = make_library_with_anchors(prefab_id, "hip_l", leg_l, "root");

        let mut app = App::new();
        app.insert_resource(library);
        app.insert_resource(descriptors);
        app.add_systems(Update, apply_motion_algorithms_on_controller_change);

        let root = app.world_mut().spawn((ObjectPrefabId(prefab_id),)).id();

        let edge = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                ObjectRefEdgeBinding {
                    root_entity: root,
                    parent_object_id: prefab_id,
                    child_object_id: leg_l,
                    attachment: Some(AttachmentDef {
                        parent_anchor: "hip_l".into(),
                        child_anchor: "root".into(),
                    }),
                    base_transform: Transform::IDENTITY,
                    base_slots: Vec::new(),
                    apply_aim_yaw: false,
                },
            ))
            .id();

        // Clear initial change ticks.
        app.update();

        app.world_mut()
            .entity_mut(root)
            .insert(MotionAlgorithmController {
                move_algorithm: MoveMotionAlgorithm::BipedWalkV1,
                ..default()
            });
        app.update();

        let player = app
            .world()
            .get::<PartAnimationPlayer>(edge)
            .expect("player inserted");
        assert!(player
            .animations
            .iter()
            .any(|s| s.channel.as_ref() == "move"));
    }
}
