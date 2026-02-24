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
    BipedMeleeSwingV1,
    QuadrupedBiteV1,
    RangedRecoilV1,
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
            Self::BipedMeleeSwingV1 => "biped_melee_swing_v1",
            Self::QuadrupedBiteV1 => "quadruped_bite_v1",
            Self::RangedRecoilV1 => "ranged_recoil_v1",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::None => "None (prefab-authored)",
            Self::BipedMeleeSwingV1 => "Biped melee swing (v1)",
            Self::QuadrupedBiteV1 => "Quadruped bite (v1)",
            Self::RangedRecoilV1 => "Ranged recoil (v1)",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "" => None,
            "none" => Some(Self::None),
            "biped_melee_swing_v1" => Some(Self::BipedMeleeSwingV1),
            "quadruped_bite_v1" => Some(Self::QuadrupedBiteV1),
            "ranged_recoil_v1" => Some(Self::RangedRecoilV1),
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
pub(crate) struct CarRigV1 {
    pub(crate) default_move_algorithm: Option<MoveMotionAlgorithm>,
    pub(crate) body: Option<MotionEdgeRefV1>,
    pub(crate) wheels: Vec<SpinEffectorV1>,
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
                .unwrap_or(MoveMotionAlgorithm::CarWheelsV1),
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
        let Some(attack_kind) = attack_kind else {
            return out;
        };
        match attack_kind {
            UnitAttackKind::RangedProjectile => {
                out.push(AttackPrimaryMotionAlgorithm::RangedRecoilV1);
            }
            UnitAttackKind::Melee => match self {
                Self::Biped(_) => out.push(AttackPrimaryMotionAlgorithm::BipedMeleeSwingV1),
                Self::Quadruped(_) => out.push(AttackPrimaryMotionAlgorithm::QuadrupedBiteV1),
                Self::Car(_) | Self::Airplane(_) => {}
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
struct CarRigV1Raw {
    wheels: Vec<SpinEffectorV1Raw>,
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
                Ok(Self::Biped(BipedRigV1 {
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
                }))
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
                Ok(Self::Car(CarRigV1 {
                    default_move_algorithm,
                    body: body.clone(),
                    wheels,
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
    PartAnimationSlot {
        channel: "move".into(),
        spec: PartAnimationSpec {
            driver: PartAnimationDriver::MoveDistance,
            speed_scale: 1.0,
            time_offset_units: 0.0,
            clip: PartAnimationDef::Spin {
                axis: axis_local,
                radians_per_unit: radians_per_meter,
            },
        },
    }
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
                    MotionRigV1::Biped(_) => AttackPrimaryMotionAlgorithm::BipedMeleeSwingV1,
                    MotionRigV1::Quadruped(_) => AttackPrimaryMotionAlgorithm::QuadrupedBiteV1,
                    MotionRigV1::Car(_) | MotionRigV1::Airplane(_) => {
                        AttackPrimaryMotionAlgorithm::None
                    }
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
        (AttackPrimaryMotionAlgorithm::BipedMeleeSwingV1, Some(MotionRigV1::Biped(_))) => algorithm,
        (AttackPrimaryMotionAlgorithm::QuadrupedBiteV1, Some(MotionRigV1::Quadruped(_))) => {
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

    let attack_override_slot = match (controller.attack_primary_algorithm, rig.as_ref()) {
        (AttackPrimaryMotionAlgorithm::None, _) => None,
        (AttackPrimaryMotionAlgorithm::RangedRecoilV1, Some(rig)) => {
            ranged_recoil_override_for_binding(rig, binding, library, attack_window_secs)
        }
        (AttackPrimaryMotionAlgorithm::BipedMeleeSwingV1, Some(MotionRigV1::Biped(rig))) => {
            biped_melee_swing_override_for_binding(rig, binding, library, attack_window_secs)
        }
        (AttackPrimaryMotionAlgorithm::QuadrupedBiteV1, Some(MotionRigV1::Quadruped(rig))) => {
            quadruped_bite_override_for_binding(rig, binding, library, attack_window_secs)
        }
        (_, _) => None,
    };

    let mut effective_slots = binding.base_slots.clone();
    if let Some(slot) = idle_override_slot {
        effective_slots.retain(|s| s.channel.as_ref() != "idle");
        effective_slots.push(slot);
    }
    if let Some(slot) = move_override_slot {
        effective_slots.retain(|s| s.channel.as_ref() != "move");
        effective_slots.push(slot);
    }
    if let Some(slot) = attack_override_slot {
        effective_slots.retain(|s| s.channel.as_ref() != "attack_primary");
        effective_slots.push(slot);
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

    let radians_per_meter = if let Some(v) = rig.radians_per_meter {
        v
    } else if let Some(radius_m) = rig.wheel_radius_m {
        1.0 / radius_m.max(0.01)
    } else {
        let size = library.size(binding.child_object_id).unwrap_or(Vec3::ONE);
        let scale = binding.base_transform.scale.abs();
        let effective = (size * scale).abs().max(Vec3::splat(0.01));
        let radius = 0.5 * effective.y.max(effective.z).max(0.01);
        (1.0 / radius).clamp(-200.0, 200.0)
    };

    Some(wheel_spin_move_slot(
        wheel.spin_axis_local,
        radians_per_meter,
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

fn biped_melee_swing_override_for_binding(
    rig: &BipedRigV1,
    binding: &ObjectRefEdgeBinding,
    library: &ObjectLibrary,
    attack_window_secs: f32,
) -> Option<PartAnimationSlot> {
    if let Some(right_arm) = rig.right_arm.as_ref() {
        if right_arm.matches_binding(binding) {
            return Some(swing_attack_slot(Vec3::X, attack_window_secs, 85.0));
        }
    }
    if let Some(left_arm) = rig.left_arm.as_ref() {
        if left_arm.matches_binding(binding) {
            return Some(swing_attack_slot(Vec3::X, attack_window_secs, 55.0));
        }
    }

    if let Some(body) = rig.body.as_ref() {
        if body.matches_binding(binding) {
            let size = library.size(binding.child_object_id).unwrap_or(Vec3::ONE);
            let scale = binding.base_transform.scale.abs();
            let effective = (size * scale).abs().max(Vec3::splat(0.01));
            let recoil = (effective.z * 0.02).clamp(0.0025, 0.08);
            return Some(recoil_attack_slot(-Vec3::Z, attack_window_secs, recoil));
        }
    }

    if let Some(head) = rig.head.as_ref() {
        if head.matches_binding(binding) {
            return Some(swing_attack_slot(Vec3::X, attack_window_secs, 18.0));
        }
    }

    None
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
