use bevy::ecs::message::MessageWriter;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use uuid::Uuid;

use crate::assets::SceneAssets;
use crate::constants::{BUILD_GRID_SIZE, BUILD_UNIT_SIZE, CROSS_BLOCK_BLOCKING_HEIGHT_FRACTION};
use crate::geometry::{clamp_world_xz, normalize_flat_direction, snap_to_grid};
use crate::object::registry::{
    ColliderProfile, MeshKey, MobilityMode, MovementBlockRule, ObjectDef, ObjectInteraction,
    ObjectLibrary, ObjectPartKind, PartAnimationDef, PartAnimationDriver, PrimitiveParams,
    PrimitiveVisualDef, UnitAttackKind,
};
use crate::object::visuals;
use crate::scene_store::SceneSaveRequest;
use crate::types::{
    AabbCollider, BuildDimensions, BuildObject, Collider, Commandable, ObjectForms, ObjectId,
    ObjectPrefabId, ObjectTint, Player,
};

use super::ai::Gen3dAiJob;
use super::state::{Gen3dDraft, Gen3dPreview, Gen3dSaveButton, Gen3dWorkshop};

#[derive(SystemParam)]
pub(crate) struct Gen3dSaveRenderWorld<'w> {
    asset_server: Res<'w, AssetServer>,
    assets: Res<'w, SceneAssets>,
    meshes: ResMut<'w, Assets<Mesh>>,
    materials: ResMut<'w, Assets<StandardMaterial>>,
    material_cache: ResMut<'w, visuals::MaterialCache>,
    mesh_cache: ResMut<'w, visuals::PrimitiveMeshCache>,
}

#[derive(SystemParam)]
pub(crate) struct Gen3dSaveEnv<'w> {
    build_scene: Res<'w, State<crate::types::BuildScene>>,
    active: Res<'w, crate::realm::ActiveRealmScene>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Gen3dSavedInstance {
    pub(crate) instance_id: ObjectId,
    pub(crate) prefab_id: u128,
    pub(crate) mobility: bool,
    pub(crate) position: Vec3,
}

#[derive(Clone, Copy, Debug)]
struct Bounds {
    min: Vec3,
    max: Vec3,
}

impl Bounds {
    fn empty() -> Self {
        Self {
            min: Vec3::splat(f32::INFINITY),
            max: Vec3::splat(f32::NEG_INFINITY),
        }
    }

    fn is_empty(self) -> bool {
        !self.min.x.is_finite() || !self.max.x.is_finite()
    }

    fn include_point(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    fn include_bounds(&mut self, other: Bounds) {
        if other.is_empty() {
            return;
        }
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
    }

    fn center(self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    fn size(self) -> Vec3 {
        (self.max - self.min).abs()
    }
}

fn saved_root_interaction(collider: ColliderProfile, collision_enabled: bool) -> ObjectInteraction {
    match collider {
        ColliderProfile::None => ObjectInteraction::none(),
        _ => ObjectInteraction {
            blocks_bullets: true,
            blocks_laser: true,
            movement_block: collision_enabled.then_some(MovementBlockRule::UpperBodyFraction(
                CROSS_BLOCK_BLOCKING_HEIGHT_FRACTION,
            )),
            supports_standing: false,
        },
    }
}

fn primitive_base_size(mesh: MeshKey, params: Option<&PrimitiveParams>) -> Vec3 {
    match mesh {
        MeshKey::UnitCapsule => match params {
            Some(PrimitiveParams::Capsule {
                half_length,
                radius,
            }) => Vec3::new(radius * 2.0, (half_length + radius) * 2.0, radius * 2.0),
            _ => Vec3::ONE,
        },
        MeshKey::UnitConicalFrustum => match params {
            Some(PrimitiveParams::ConicalFrustum {
                radius_top,
                radius_bottom,
                height,
            }) => {
                let r = radius_top.max(*radius_bottom);
                Vec3::new(r * 2.0, *height, r * 2.0)
            }
            _ => Vec3::ONE,
        },
        MeshKey::UnitTorus => match params {
            Some(PrimitiveParams::Torus {
                minor_radius,
                major_radius,
            }) => {
                let r = major_radius + minor_radius;
                Vec3::new(r * 2.0, minor_radius * 2.0, r * 2.0)
            }
            _ => Vec3::ONE,
        },
        _ => Vec3::ONE,
    }
}

fn anchor_transform(def: &ObjectDef, name: &str) -> Option<Transform> {
    if name == "origin" {
        return Some(Transform::IDENTITY);
    }
    def.anchors
        .iter()
        .find(|a| a.name.as_ref() == name)
        .map(|a| a.transform)
}

fn bounds_of_object(
    object_id: u128,
    defs: &std::collections::HashMap<u128, ObjectDef>,
    stack: &mut Vec<u128>,
    memo: &mut std::collections::HashMap<u128, Bounds>,
) -> Bounds {
    fn compose_transform(a: Transform, b: Transform) -> Option<Transform> {
        let composed = a.to_matrix() * b.to_matrix();
        crate::geometry::mat4_to_transform_allow_degenerate_scale(composed)
    }

    fn part_transform_samples(part: &crate::object::registry::ObjectPartDef) -> Vec<Transform> {
        let mut out = Vec::new();
        out.push(part.transform);

        for slot in part.animations.iter() {
            match &slot.spec.clip {
                PartAnimationDef::Loop { keyframes, .. }
                | PartAnimationDef::Once { keyframes, .. }
                | PartAnimationDef::PingPong { keyframes, .. } => {
                    for keyframe in keyframes {
                        if let Some(t) = compose_transform(part.transform, keyframe.delta) {
                            out.push(t);
                        }
                    }
                }
                PartAnimationDef::Spin { axis, .. } => {
                    let axis = if axis.length_squared() > 1e-6 {
                        axis.normalize()
                    } else {
                        Vec3::Y
                    };
                    for i in 0..4 {
                        let angle = (i as f32) * core::f32::consts::FRAC_PI_2;
                        let delta = Transform {
                            rotation: Quat::from_axis_angle(axis, angle),
                            ..default()
                        };
                        if let Some(t) = compose_transform(part.transform, delta) {
                            out.push(t);
                        }
                    }
                }
            }
        }

        out
    }

    if let Some(cached) = memo.get(&object_id) {
        return *cached;
    }
    if stack.contains(&object_id) {
        return Bounds::empty();
    }
    let Some(def) = defs.get(&object_id) else {
        return Bounds::empty();
    };

    stack.push(object_id);
    let mut bounds = Bounds::empty();

    for part in def.parts.iter() {
        let samples = part_transform_samples(part);
        match &part.kind {
            ObjectPartKind::Primitive { primitive } => {
                let (mesh, params) = match primitive {
                    PrimitiveVisualDef::Primitive { mesh, params, .. } => (*mesh, params.as_ref()),
                    PrimitiveVisualDef::Mesh { mesh, material } => {
                        let _ = material;
                        (*mesh, None)
                    }
                };

                let base = primitive_base_size(mesh, params);
                for sample in samples {
                    let local_half = (base * sample.scale).abs() * 0.5;

                    let abs = Mat3::from_quat(sample.rotation).abs();
                    let ext = abs * local_half;
                    let center = sample.translation;
                    bounds.include_point(center - ext);
                    bounds.include_point(center + ext);
                }
            }
            ObjectPartKind::ObjectRef { object_id: child } => {
                let child_bounds = bounds_of_object(*child, defs, stack, memo);
                if child_bounds.is_empty() {
                    continue;
                }

                for sample in samples {
                    let child_mat = if let Some(attachment) = part.attachment.as_ref() {
                        let parent_anchor =
                            anchor_transform(def, attachment.parent_anchor.as_ref())
                                .unwrap_or(Transform::IDENTITY);
                        let child_anchor = defs
                            .get(child)
                            .and_then(|child_def| {
                                anchor_transform(child_def, attachment.child_anchor.as_ref())
                            })
                            .unwrap_or(Transform::IDENTITY);

                        parent_anchor.to_matrix()
                            * sample.to_matrix()
                            * child_anchor.to_matrix().inverse()
                    } else {
                        sample.to_matrix()
                    };

                    let corners = [
                        Vec3::new(child_bounds.min.x, child_bounds.min.y, child_bounds.min.z),
                        Vec3::new(child_bounds.min.x, child_bounds.min.y, child_bounds.max.z),
                        Vec3::new(child_bounds.min.x, child_bounds.max.y, child_bounds.min.z),
                        Vec3::new(child_bounds.min.x, child_bounds.max.y, child_bounds.max.z),
                        Vec3::new(child_bounds.max.x, child_bounds.min.y, child_bounds.min.z),
                        Vec3::new(child_bounds.max.x, child_bounds.min.y, child_bounds.max.z),
                        Vec3::new(child_bounds.max.x, child_bounds.max.y, child_bounds.min.z),
                        Vec3::new(child_bounds.max.x, child_bounds.max.y, child_bounds.max.z),
                    ];

                    let mut transformed = Bounds::empty();
                    for corner in corners {
                        transformed.include_point(child_mat.transform_point3(corner));
                    }
                    bounds.include_bounds(transformed);
                }
            }
            ObjectPartKind::Model { .. } => {}
        };
    }

    stack.pop();
    memo.insert(object_id, bounds);
    bounds
}

fn bounds_of_primitive_parts_only(def: &ObjectDef) -> Bounds {
    let mut bounds = Bounds::empty();
    for part in def.parts.iter() {
        let ObjectPartKind::Primitive { primitive } = &part.kind else {
            continue;
        };
        let (mesh, params) = match primitive {
            PrimitiveVisualDef::Primitive { mesh, params, .. } => (*mesh, params.as_ref()),
            PrimitiveVisualDef::Mesh { mesh, material } => {
                let _ = material;
                (*mesh, None)
            }
        };

        let base = primitive_base_size(mesh, params);
        let local_half = (base * part.transform.scale).abs() * 0.5;
        let center = part.transform.translation;
        let rot = part.transform.rotation;
        if !center.is_finite() || !local_half.is_finite() || !rot.is_finite() {
            continue;
        }

        let abs = Mat3::from_quat(rot).abs();
        let ext = abs * local_half;
        bounds.include_point(center - ext);
        bounds.include_point(center + ext);
    }
    bounds
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::registry::{AnchorDef, AttachmentDef, ObjectPartDef};
    use crate::object::registry::{
        PartAnimationDef, PartAnimationDriver, PartAnimationKeyframeDef, PartAnimationSpec,
    };

    #[test]
    fn bounds_of_object_respects_attachment_anchors() {
        let parent_id = 1u128;
        let child_id = 2u128;

        let child_def = ObjectDef {
            object_id: child_id,
            label: "child".into(),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: vec![AnchorDef {
                name: "plug".into(),
                transform: Transform::from_xyz(0.0, 2.0, 0.0),
            }],
            parts: vec![ObjectPartDef::primitive(
                PrimitiveVisualDef::Primitive {
                    mesh: MeshKey::UnitCube,
                    params: None,
                    color: Color::srgb(1.0, 1.0, 1.0),
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

        let mut child_ref = ObjectPartDef::object_ref(child_id, Transform::IDENTITY);
        child_ref = child_ref.with_attachment(AttachmentDef {
            parent_anchor: "socket".into(),
            child_anchor: "plug".into(),
        });

        let parent_def = ObjectDef {
            object_id: parent_id,
            label: "parent".into(),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: vec![AnchorDef {
                name: "socket".into(),
                transform: Transform::from_xyz(0.0, -2.0, 0.0),
            }],
            parts: vec![child_ref],
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        };

        let mut defs = std::collections::HashMap::new();
        defs.insert(parent_id, parent_def);
        defs.insert(child_id, child_def);
        let mut memo = std::collections::HashMap::<u128, Bounds>::new();
        let mut stack = Vec::new();

        let bounds = bounds_of_object(parent_id, &defs, &mut stack, &mut memo);
        assert!(!bounds.is_empty());
        // Child plug at +2y is aligned to parent socket at -2y, so child's origin is at -4y.
        // Child cube extends to y=-4.5..-3.5.
        assert!((bounds.min.y + 4.5).abs() < 1e-3, "min.y={}", bounds.min.y);
        assert!((bounds.max.y + 3.5).abs() < 1e-3, "max.y={}", bounds.max.y);
    }

    #[test]
    fn bounds_of_object_includes_part_animation_keyframes() {
        use crate::object::registry::{PartAnimationSlot, PrimitiveVisualDef};

        let parent_id = 1u128;
        let child_id = 2u128;

        let child_def = ObjectDef {
            object_id: child_id,
            label: "child".into(),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: Vec::new(),
            parts: vec![ObjectPartDef::primitive(
                PrimitiveVisualDef::Primitive {
                    mesh: MeshKey::UnitCube,
                    params: None,
                    color: Color::srgb(1.0, 1.0, 1.0),
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

        let mut child_ref = ObjectPartDef::object_ref(child_id, Transform::IDENTITY);
        child_ref.animations.push(PartAnimationSlot {
            channel: "move".into(),
            spec: PartAnimationSpec {
                driver: PartAnimationDriver::MovePhase,
                speed_scale: 1.0,
                time_offset_units: 0.0,
                clip: PartAnimationDef::Loop {
                    duration_secs: 1.0,
                    keyframes: vec![PartAnimationKeyframeDef {
                        time_secs: 0.0,
                        delta: Transform::from_translation(Vec3::new(0.0, -2.0, 0.0)),
                    }],
                },
            },
        });

        let parent_def = ObjectDef {
            object_id: parent_id,
            label: "parent".into(),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: Vec::new(),
            parts: vec![child_ref],
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        };

        let mut defs = std::collections::HashMap::new();
        defs.insert(parent_id, parent_def);
        defs.insert(child_id, child_def);
        let mut memo = std::collections::HashMap::<u128, Bounds>::new();
        let mut stack = Vec::new();

        let bounds = bounds_of_object(parent_id, &defs, &mut stack, &mut memo);
        assert!(!bounds.is_empty());
        assert!(
            (bounds.min.y + 2.5).abs() < 1e-3,
            "expected min.y≈-2.5, got {}",
            bounds.min.y
        );
        assert!(
            (bounds.max.y - 0.5).abs() < 1e-3,
            "expected max.y≈0.5, got {}",
            bounds.max.y
        );

        // Extra sanity: should still include the child's origin pose (y=-0.5..0.5).
        assert!(bounds.min.y <= -2.5 + 1e-3);
        assert!(bounds.max.y >= 0.5 - 1e-3);
        assert!(memo.contains_key(&parent_id));
    }

    #[test]
    fn draft_to_saved_defs_preserves_root_unit_collider_profile() {
        use crate::object::registry::MobilityDef;
        use crate::object::registry::ObjectPartDef;

        let root_id = super::super::gen3d_draft_object_id();
        let body_root_id = 0x10u128;
        let torso_id = 0x11u128;

        let torso_def = ObjectDef {
            object_id: torso_id,
            label: "torso".into(),
            size: Vec3::new(1.3, 1.0, 1.3),
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: Vec::new(),
            parts: vec![ObjectPartDef::primitive(
                PrimitiveVisualDef::Primitive {
                    mesh: MeshKey::UnitCube,
                    params: None,
                    color: Color::srgb(1.0, 0.0, 0.0),
                    unlit: false,
                },
                Transform::from_scale(Vec3::new(1.3, 1.0, 1.3)),
            )],
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        };

        let body_root_def = ObjectDef {
            object_id: body_root_id,
            label: "body_root".into(),
            size: Vec3::new(0.3, 0.2, 0.3),
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: Vec::new(),
            parts: vec![
                ObjectPartDef::primitive(
                    PrimitiveVisualDef::Primitive {
                        mesh: MeshKey::UnitCube,
                        params: None,
                        color: Color::srgba(0.0, 0.0, 0.0, 0.0),
                        unlit: true,
                    },
                    Transform::from_scale(Vec3::new(0.3, 0.2, 0.3)),
                ),
                ObjectPartDef::object_ref(torso_id, Transform::from_translation(Vec3::X * 0.8)),
            ],
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        };

        let root_def = ObjectDef {
            object_id: root_id,
            label: "gen3d_draft".into(),
            size: Vec3::new(1.3, 1.0, 1.3),
            ground_origin_y: None,
            collider: ColliderProfile::CircleXZ { radius: 0.65 },
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: Some(MobilityDef {
                mode: MobilityMode::Ground,
                max_speed: 1.0,
            }),
            anchors: Vec::new(),
            parts: vec![ObjectPartDef::object_ref(body_root_id, Transform::IDENTITY)
                .with_attachment(AttachmentDef {
                    parent_anchor: "origin".into(),
                    child_anchor: "origin".into(),
                })],
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        };

        let draft = Gen3dDraft {
            defs: vec![root_def, body_root_def, torso_def],
        };
        let (saved_root_id, saved_defs) =
            draft_to_saved_defs(&draft, false, None).expect("save ok");
        let saved_root = saved_defs
            .iter()
            .find(|def| def.object_id == saved_root_id)
            .expect("saved root present");
        match saved_root.collider {
            ColliderProfile::CircleXZ { radius } => {
                assert!((radius - 0.65).abs() < 1e-6, "radius={radius}");
            }
            other => panic!("expected CircleXZ collider, got {other:?}"),
        }
    }

    #[test]
    fn draft_to_saved_defs_building_root_collision_respects_collision_enabled_flag() {
        use crate::object::registry::{MovementBlockRule, ObjectPartDef};

        let root_id = super::super::gen3d_draft_object_id();
        let body_id = 0x10u128;

        let body_def = ObjectDef {
            object_id: body_id,
            label: "body".into(),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: Vec::new(),
            parts: vec![ObjectPartDef::primitive(
                PrimitiveVisualDef::Primitive {
                    mesh: MeshKey::UnitCube,
                    params: None,
                    color: Color::srgb(1.0, 1.0, 1.0),
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

        let root_def = ObjectDef {
            object_id: root_id,
            label: "gen3d_draft".into(),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: Vec::new(),
            parts: vec![ObjectPartDef::object_ref(body_id, Transform::IDENTITY)],
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        };

        let draft = Gen3dDraft {
            defs: vec![root_def, body_def],
        };

        assert!(
            draft_to_saved_defs(&draft, false, None)
                .ok()
                .and_then(|(saved_root_id, saved_defs)| {
                    saved_defs
                        .iter()
                        .find(|def| def.object_id == saved_root_id)
                        .map(|def| def.interaction.movement_block)
                })
                .flatten()
                .is_none(),
            "expected collision-disabled Gen3D building roots to not block movement",
        );

        let saved_root_movement_block = draft_to_saved_defs(&draft, true, None)
            .ok()
            .and_then(|(saved_root_id, saved_defs)| {
                saved_defs
                    .iter()
                    .find(|def| def.object_id == saved_root_id)
                    .map(|def| def.interaction.movement_block)
            })
            .flatten();

        match saved_root_movement_block {
            Some(MovementBlockRule::UpperBodyFraction(fraction)) => {
                assert!(
                    (fraction - CROSS_BLOCK_BLOCKING_HEIGHT_FRACTION).abs() < 1e-6,
                    "fraction={fraction}",
                );
            }
            other => panic!("expected UpperBodyFraction movement_block, got {other:?}"),
        };
    }
}

pub(super) fn draft_to_saved_defs(
    draft: &Gen3dDraft,
    collision_enabled: bool,
    root_prefab_id_override: Option<u128>,
) -> Result<(u128, Vec<ObjectDef>), String> {
    let root_id = super::gen3d_draft_object_id();
    let Some(root_def) = draft.defs.iter().find(|d| d.object_id == root_id) else {
        return Err("Gen3D: missing root draft object def.".into());
    };
    let root_is_unit = root_def.mobility.is_some();

    let defs_map: std::collections::HashMap<u128, ObjectDef> = draft
        .defs
        .iter()
        .map(|d| (d.object_id, d.clone()))
        .collect();
    let mut memo = std::collections::HashMap::<u128, Bounds>::new();
    let mut stack = Vec::new();
    let root_bounds = bounds_of_object(root_id, &defs_map, &mut stack, &mut memo);
    let root_size_override =
        (!root_bounds.is_empty()).then(|| root_bounds.size().abs().max(Vec3::splat(0.01)));

    let mut recenter = Vec3::ZERO;
    let mut root_ground_origin_y = None;
    if root_is_unit {
        let root_ref = root_def.parts.iter().find_map(|part| match &part.kind {
            ObjectPartKind::ObjectRef { object_id } => Some((part, *object_id)),
            _ => None,
        });

        if let Some((root_ref, root_component_id)) = root_ref {
            if let Some(root_component_def) = defs_map.get(&root_component_id) {
                let bounds = bounds_of_primitive_parts_only(root_component_def);
                if !bounds.is_empty() {
                    let torso_center_local = bounds.center();
                    recenter = root_ref
                        .transform
                        .to_matrix()
                        .transform_point3(torso_center_local);
                }
            }
        }

        if !root_bounds.is_empty() {
            // After applying `recenter`, the new bounds are `root_bounds - recenter`, so:
            // `ground_origin_y = -min.y`.
            root_ground_origin_y = Some((recenter.y - root_bounds.min.y).max(0.0));
        }
    } else if !root_bounds.is_empty() {
        recenter = root_bounds.center();
    }

    let mut id_map = std::collections::HashMap::<u128, u128>::new();
    for def in &draft.defs {
        let new_id = if def.object_id == root_id {
            root_prefab_id_override.unwrap_or_else(|| Uuid::new_v4().as_u128())
        } else {
            Uuid::new_v4().as_u128()
        };
        id_map.insert(def.object_id, new_id);
    }
    let saved_root_id = *id_map
        .get(&root_id)
        .ok_or_else(|| "Gen3D: internal error: missing root id mapping.".to_string())?;

    let mut out_defs: Vec<ObjectDef> = Vec::with_capacity(draft.defs.len());
    for def in &draft.defs {
        let Some(new_id) = id_map.get(&def.object_id).copied() else {
            continue;
        };

        let mut new_def = def.clone();
        new_def.object_id = new_id;

        // Give the saved root a stable-ish label for debugging.
        if def.object_id == root_id {
            new_def.label = format!("Gen3DModel_{:08x}", (saved_root_id >> 96) as u32).into();
            if let Some(size) = root_size_override {
                new_def.size = size;
            }

            if root_is_unit {
                if let Some(ground_origin_y) = root_ground_origin_y {
                    new_def.ground_origin_y = Some(ground_origin_y);
                }
            } else if let Some(size) = root_size_override {
                new_def.collider = match new_def.collider {
                    ColliderProfile::CircleXZ { .. } => ColliderProfile::CircleXZ {
                        radius: (size.x.max(size.z) * 0.5).max(0.01),
                    },
                    ColliderProfile::None => ColliderProfile::AabbXZ {
                        half_extents: Vec2::new(size.x * 0.5, size.z * 0.5),
                    },
                    _ => ColliderProfile::AabbXZ {
                        half_extents: Vec2::new(size.x * 0.5, size.z * 0.5),
                    },
                };
            }

            new_def.interaction = if new_def.mobility.is_some() {
                ObjectInteraction::none()
            } else {
                saved_root_interaction(new_def.collider, collision_enabled)
            };
        }

        for part in &mut new_def.parts {
            if let ObjectPartKind::ObjectRef { object_id } = &mut part.kind {
                if let Some(mapped) = id_map.get(object_id) {
                    *object_id = *mapped;
                }
            }
        }

        if let Some(attack) = new_def.attack.as_mut() {
            match attack.kind {
                UnitAttackKind::Melee => {}
                UnitAttackKind::RangedProjectile => {
                    if let Some(ranged) = attack.ranged.as_mut() {
                        if let Some(mapped) = id_map.get(&ranged.projectile_prefab) {
                            ranged.projectile_prefab = *mapped;
                        }
                        if let Some(mapped) = id_map.get(&ranged.muzzle.object_id) {
                            ranged.muzzle.object_id = *mapped;
                        }
                    }
                }
            }
        }

        if let Some(aim) = new_def.aim.as_mut() {
            for component_id in aim.components.iter_mut() {
                if let Some(mapped) = id_map.get(component_id) {
                    *component_id = *mapped;
                }
            }
        }

        if def.object_id == root_id {
            for part in &mut new_def.parts {
                part.transform.translation -= recenter;
            }
        }

        out_defs.push(new_def);
    }

    Ok((saved_root_id, out_defs))
}

fn collider_half_xz(collider: ColliderProfile, size: Vec3) -> Vec2 {
    match collider {
        ColliderProfile::AabbXZ { half_extents } => half_extents,
        ColliderProfile::CircleXZ { radius } => Vec2::splat(radius),
        ColliderProfile::None => Vec2::new(size.x * 0.5, size.z * 0.5),
    }
}

fn save_gen3d_snapshot_to_scene_and_library(
    realm_id: &str,
    scene_id: &str,
    library: &mut ObjectLibrary,
    prefab_descriptors: Option<&mut crate::prefab_descriptors::PrefabDescriptorLibrary>,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    snapshot: &Gen3dDraft,
    collision_enabled: bool,
) -> Result<(u128, ObjectDef), String> {
    let (saved_root_id, defs) =
        draft_to_saved_defs(snapshot, collision_enabled, job.save_overwrite_prefab_id())?;
    let scene_prefabs_dir = crate::scene_prefabs::save_scene_prefab_package_defs(
        realm_id,
        scene_id,
        saved_root_id,
        &defs,
    )?;

    save_gen3d_source_bundle_best_effort(
        &crate::scene_prefabs::scene_prefab_package_gen3d_source_dir(
            realm_id,
            scene_id,
            saved_root_id,
        ),
        snapshot,
    );
    save_gen3d_edit_bundle_best_effort(
        &crate::scene_prefabs::scene_prefab_package_gen3d_edit_bundle_path(
            realm_id,
            scene_id,
            saved_root_id,
        ),
        job,
        saved_root_id,
    );

    for def in defs {
        library.upsert(def);
    }

    let Some(root_def) = library.get(saved_root_id).cloned() else {
        return Err("Cannot save: missing saved prefab def.".into());
    };

    let descriptor = save_generated_prefab_descriptor_best_effort(
        &scene_prefabs_dir,
        &root_def,
        library,
        job,
        workshop,
    );
    if let Some(prefab_descriptors) = prefab_descriptors {
        prefab_descriptors.upsert(saved_root_id, descriptor);
    }

    Ok((saved_root_id, root_def))
}

fn collect_descendants(root: Entity, children_q: &Query<&Children>) -> Vec<Entity> {
    let mut stack = vec![root];
    let mut out: Vec<Entity> = Vec::new();
    while let Some(entity) = stack.pop() {
        let Ok(children) = children_q.get(entity) else {
            continue;
        };
        for child in children.iter() {
            out.push(child);
            stack.push(child);
        }
    }
    out
}

fn refresh_object_visuals_for_root(
    commands: &mut Commands,
    library: &ObjectLibrary,
    asset_server: &AssetServer,
    assets: &SceneAssets,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    material_cache: &mut visuals::MaterialCache,
    mesh_cache: &mut visuals::PrimitiveMeshCache,
    root_entity: Entity,
    prefab_id: u128,
    tint: Option<Color>,
    children_q: &Query<&Children>,
) {
    let descendants = collect_descendants(root_entity, children_q);
    for entity in descendants.into_iter().rev() {
        commands.entity(entity).try_despawn();
    }

    let mut ec = commands.entity(root_entity);
    visuals::spawn_object_visuals(
        &mut ec,
        library,
        asset_server,
        assets,
        meshes,
        materials,
        material_cache,
        mesh_cache,
        prefab_id,
        tint,
    );
}

fn gen3d_save_seeded_session_in_place(
    commands: &mut Commands,
    asset_server: &AssetServer,
    assets: &SceneAssets,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    material_cache: &mut visuals::MaterialCache,
    mesh_cache: &mut visuals::PrimitiveMeshCache,
    realm_id: &str,
    scene_id: &str,
    library: &mut ObjectLibrary,
    prefab_descriptors: &mut crate::prefab_descriptors::PrefabDescriptorLibrary,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &Gen3dDraft,
    collision_enabled: bool,
    target_entity: Entity,
    world_objects: &Query<
        (
            Entity,
            &ObjectId,
            &Transform,
            &ObjectPrefabId,
            Option<&ObjectTint>,
        ),
        (Without<Player>, Or<(With<BuildObject>, With<Commandable>)>),
    >,
    children_q: &Query<&Children>,
    scene_saves: &mut MessageWriter<SceneSaveRequest>,
) -> Result<Gen3dSavedInstance, String> {
    if draft.root_def().is_none() || draft.total_non_projectile_primitive_parts() == 0 {
        return Err("Cannot save: draft is empty.".into());
    }

    let base_prefab_id = job.edit_base_prefab_id().unwrap_or(0);
    let (target_instance_id, target_tint, target_pos) = match world_objects.get(target_entity) {
        Ok((_entity, instance_id, transform, _prefab_id, tint)) => {
            (*instance_id, tint.map(|t| t.0), transform.translation)
        }
        Err(_) => {
            return Err("Cannot save: missing target instance (it may have been deleted).".into());
        }
    };

    // Snapshot at call time so a concurrent Build run can't mutate it mid-save.
    let snapshot = Gen3dDraft {
        defs: draft.defs.clone(),
    };
    let overwrite_prefab_id = job.save_overwrite_prefab_id();
    let (saved_root_id, root_def) = save_gen3d_snapshot_to_scene_and_library(
        realm_id,
        scene_id,
        library,
        Some(prefab_descriptors),
        workshop,
        job,
        &snapshot,
        collision_enabled,
    )?;

    let size = root_def.size;
    let half_xz = collider_half_xz(root_def.collider, size);
    let object_radius = half_xz.x.max(half_xz.y).max(0.1);
    let mobility = root_def.mobility.is_some();

    let mut updated_instances = 0usize;
    if overwrite_prefab_id.is_some() {
        // Edit (overwrite): refresh visuals for all instances of this prefab id.
        for (entity, _instance_id, _transform, prefab_id, tint) in world_objects {
            if prefab_id.0 != saved_root_id {
                continue;
            }
            refresh_object_visuals_for_root(
                commands,
                library,
                asset_server,
                assets,
                meshes,
                materials,
                material_cache,
                mesh_cache,
                entity,
                saved_root_id,
                tint.map(|t| t.0),
                children_q,
            );
            if mobility {
                commands.entity(entity).insert(Collider {
                    radius: object_radius,
                });
            } else {
                commands.entity(entity).insert(BuildDimensions { size });
                commands.entity(entity).insert(AabbCollider {
                    half_extents: half_xz,
                });
            }
            updated_instances += 1;
        }

        workshop.status = format!(
            "Saved prefab to the scene (overwrote prefab). Updated {updated_instances} instance(s) in the world. Exit Gen3D to inspect."
        );
        scene_saves.write(SceneSaveRequest::new("Gen3D saved prefab (edit overwrite)"));
    } else {
        // Fork: bind only the selected instance to the new prefab id.
        if !mobility {
            return Err("Fork save expects a unit prefab (mobility=true).".into());
        }

        commands
            .entity(target_entity)
            .insert(ObjectPrefabId(saved_root_id));
        commands
            .entity(target_entity)
            .insert(ObjectForms::new_single(saved_root_id));
        commands.entity(target_entity).insert(Collider {
            radius: object_radius,
        });

        refresh_object_visuals_for_root(
            commands,
            library,
            asset_server,
            assets,
            meshes,
            materials,
            material_cache,
            mesh_cache,
            target_entity,
            saved_root_id,
            target_tint,
            children_q,
        );
        updated_instances = 1;

        // After the initial fork save, treat subsequent saves as "edit the fork" (overwrite).
        job.set_edit_base_prefab_id(Some(saved_root_id));
        job.set_save_overwrite_prefab_id(Some(saved_root_id));

        workshop.status =
            "Saved forked prefab to the scene and updated the selected instance. Exit Gen3D to inspect."
                .into();
        scene_saves.write(SceneSaveRequest::new("Gen3D forked model"));
    }

    workshop.error = None;

    // Persist a small save artifact for debugging / correlation with agent runs.
    let save_seq = job.bump_save_seq();
    if let Some(run_dir) = job.run_dir_path() {
        let created_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let artifact = serde_json::json!({
            "version": 1,
            "created_at_ms": created_at_ms,
            "save_seq": save_seq,
            "kind": if overwrite_prefab_id.is_some() { "edit_overwrite" } else { "fork_rebind" },
            "run_id": job.run_id().map(|id| id.to_string()),
            "attempt": job.attempt(),
            "pass": job.pass(),
            "plan_hash": job.plan_hash(),
            "assembly_rev": job.assembly_rev(),
            "workspace_id": job.active_workspace_id(),
            "base_prefab_id_uuid": uuid::Uuid::from_u128(base_prefab_id).to_string(),
            "saved_root_id_uuid": uuid::Uuid::from_u128(saved_root_id).to_string(),
            "target_instance_id_uuid": uuid::Uuid::from_u128(target_instance_id.0).to_string(),
            "updated_instances": updated_instances,
            "mobility": mobility,
            "target_world_pos": [target_pos.x, target_pos.y, target_pos.z],
        });
        let path = run_dir.join(format!("save_{save_seq:04}.json"));
        if let Err(err) = std::fs::write(
            &path,
            serde_json::to_string_pretty(&artifact).unwrap_or_else(|_| artifact.to_string()),
        ) {
            warn!(
                "Gen3D: failed to write save artifact {}: {err}",
                path.display()
            );
        }
    }

    Ok(Gen3dSavedInstance {
        instance_id: target_instance_id,
        prefab_id: saved_root_id,
        mobility,
        position: target_pos,
    })
}

pub(crate) fn gen3d_save_button(
    env: Gen3dSaveEnv,
    mut commands: Commands,
    mut render: Gen3dSaveRenderWorld,
    mut library: ResMut<ObjectLibrary>,
    mut prefab_descriptors: ResMut<crate::prefab_descriptors::PrefabDescriptorLibrary>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut model_library: ResMut<crate::model_library_ui::ModelLibraryUiState>,
    mut job: ResMut<Gen3dAiJob>,
    draft: Res<Gen3dDraft>,
    preview: Res<Gen3dPreview>,
    player_q: Query<(&Transform, &Collider), With<Player>>,
    world_objects: Query<
        (
            Entity,
            &ObjectId,
            &Transform,
            &ObjectPrefabId,
            Option<&ObjectTint>,
        ),
        (Without<Player>, Or<(With<BuildObject>, With<Commandable>)>),
    >,
    children_q: Query<&Children>,
    mut scene_saves: MessageWriter<SceneSaveRequest>,
    mut last_interaction: Local<Option<Interaction>>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        With<Gen3dSaveButton>,
    >,
) {
    if !matches!(env.build_scene.get(), crate::types::BuildScene::Preview) {
        return;
    }

    let enabled = draft.root_def().is_some() && draft.total_non_projectile_primitive_parts() > 0;

    let Ok((interaction, mut bg, mut border)) = buttons.single_mut() else {
        *last_interaction = None;
        return;
    };

    if !enabled {
        *bg = BackgroundColor(Color::srgba(0.10, 0.10, 0.11, 0.55));
        *border = BorderColor::all(Color::srgba(0.30, 0.30, 0.34, 0.70));
        *last_interaction = Some(*interaction);
        return;
    }

    match *interaction {
        Interaction::None => {
            *bg = BackgroundColor(Color::srgba(0.06, 0.10, 0.16, 0.80));
            *border = BorderColor::all(Color::srgb(0.30, 0.55, 0.95));
        }
        Interaction::Hovered => {
            *bg = BackgroundColor(Color::srgba(0.08, 0.13, 0.20, 0.88));
            *border = BorderColor::all(Color::srgb(0.35, 0.60, 1.00));
        }
        Interaction::Pressed => {
            *bg = BackgroundColor(Color::srgba(0.10, 0.16, 0.25, 0.96));
            *border = BorderColor::all(Color::srgb(0.40, 0.65, 1.00));

            let was_pressed = matches!(*last_interaction, Some(Interaction::Pressed));
            if was_pressed {
                return;
            }

            let Ok((player_transform, player_collider)) = player_q.single() else {
                workshop.error = Some("Cannot save: missing hero entity.".into());
                workshop.status = "Save failed.".into();
                return;
            };

            match gen3d_save_current_draft_seed_aware_from_api(
                &mut commands,
                &render.asset_server,
                &render.assets,
                &mut *render.meshes,
                &mut *render.materials,
                &mut *render.material_cache,
                &mut *render.mesh_cache,
                &env.active.realm_id,
                &env.active.scene_id,
                &mut library,
                &mut *prefab_descriptors,
                &mut workshop,
                &mut job,
                &draft,
                preview.show_collision,
                player_transform,
                player_collider,
                &world_objects,
                &children_q,
                &mut scene_saves,
            ) {
                Ok(_) => model_library.mark_models_dirty(),
                Err(err) => {
                    workshop.error = Some(err);
                    workshop.status = "Save failed.".into();
                }
            }
        }
    }

    *last_interaction = Some(*interaction);
}

pub(crate) fn gen3d_save_current_draft_seed_aware_from_api(
    commands: &mut Commands,
    asset_server: &AssetServer,
    assets: &SceneAssets,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    material_cache: &mut visuals::MaterialCache,
    mesh_cache: &mut visuals::PrimitiveMeshCache,
    realm_id: &str,
    scene_id: &str,
    library: &mut ObjectLibrary,
    prefab_descriptors: &mut crate::prefab_descriptors::PrefabDescriptorLibrary,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &Gen3dDraft,
    collision_enabled: bool,
    player_transform: &Transform,
    player_collider: &Collider,
    world_objects: &Query<
        (
            Entity,
            &ObjectId,
            &Transform,
            &ObjectPrefabId,
            Option<&ObjectTint>,
        ),
        (Without<Player>, Or<(With<BuildObject>, With<Commandable>)>),
    >,
    children_q: &Query<&Children>,
    scene_saves: &mut MessageWriter<SceneSaveRequest>,
) -> Result<Gen3dSavedInstance, String> {
    let seeded_target = job
        .seed_target_entity()
        .zip(job.edit_base_prefab_id())
        .map(|(entity, _)| entity);

    if let Some(target_entity) = seeded_target {
        gen3d_save_seeded_session_in_place(
            commands,
            asset_server,
            assets,
            meshes,
            materials,
            material_cache,
            mesh_cache,
            realm_id,
            scene_id,
            library,
            prefab_descriptors,
            workshop,
            job,
            draft,
            collision_enabled,
            target_entity,
            world_objects,
            children_q,
            scene_saves,
        )
    } else {
        gen3d_save_current_draft_from_api(
            commands,
            asset_server,
            assets,
            meshes,
            materials,
            material_cache,
            mesh_cache,
            realm_id,
            scene_id,
            library,
            Some(prefab_descriptors),
            workshop,
            job,
            draft,
            collision_enabled,
            player_transform,
            player_collider,
            scene_saves,
        )
    }
}

pub(crate) fn gen3d_save_current_draft_from_api(
    commands: &mut Commands,
    asset_server: &AssetServer,
    assets: &SceneAssets,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    material_cache: &mut visuals::MaterialCache,
    mesh_cache: &mut visuals::PrimitiveMeshCache,
    realm_id: &str,
    scene_id: &str,
    library: &mut ObjectLibrary,
    prefab_descriptors: Option<&mut crate::prefab_descriptors::PrefabDescriptorLibrary>,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &Gen3dDraft,
    collision_enabled: bool,
    player_transform: &Transform,
    player_collider: &Collider,
    scene_saves: &mut MessageWriter<SceneSaveRequest>,
) -> Result<Gen3dSavedInstance, String> {
    if draft.root_def().is_none() || draft.total_non_projectile_primitive_parts() == 0 {
        return Err("Cannot save: draft is empty.".into());
    }

    // Snapshot the draft at call time so a concurrent Build run can't mutate it mid-save.
    let snapshot = Gen3dDraft {
        defs: draft.defs.clone(),
    };
    let (saved_root_id, root_def) = save_gen3d_snapshot_to_scene_and_library(
        realm_id,
        scene_id,
        library,
        prefab_descriptors,
        workshop,
        job,
        &snapshot,
        collision_enabled,
    )?;

    let size = root_def.size;
    let half_xz = collider_half_xz(root_def.collider, size);
    let object_radius = half_xz.x.max(half_xz.y).max(0.1);
    let mobility = root_def.mobility.is_some();
    let mobility_mode = root_def.mobility.map(|m| m.mode);

    let forward = normalize_flat_direction(player_transform.rotation * Vec3::Z).unwrap_or(Vec3::Z);
    let right = Vec3::Y.cross(forward).normalize_or_zero();
    let distance = player_collider.radius + object_radius + BUILD_UNIT_SIZE;

    // Avoid stacking multiple saved models on top of each other: scatter spawn positions around
    // the hero deterministically using the current save sequence.
    //
    // This keeps newly saved units/buildings visible and makes locomotion animation checks easier.
    let save_slot = job.current_save_seq();
    let slots_per_ring: u32 = 12;
    let ring = save_slot / slots_per_ring;
    let index_in_ring = save_slot % slots_per_ring;
    let angle = (index_in_ring as f32) * (std::f32::consts::TAU / slots_per_ring as f32);
    let mut dir = (right * angle.cos() + forward * angle.sin()).normalize_or_zero();
    if dir.length_squared() <= 0.0001 {
        dir = Vec3::X;
    }
    let spacing = (object_radius * 2.0 + BUILD_UNIT_SIZE * 2.0).max(BUILD_UNIT_SIZE * 4.0);
    let radial = distance + ring as f32 * spacing;

    let mut pos = player_transform.translation + dir * radial;
    pos.x = snap_to_grid(pos.x, BUILD_GRID_SIZE);
    pos.z = snap_to_grid(pos.z, BUILD_GRID_SIZE);
    let ground_y = library.ground_origin_y_or_default(saved_root_id);
    pos.y = match mobility_mode {
        Some(MobilityMode::Air) => ground_y + BUILD_UNIT_SIZE * 8.0,
        _ => ground_y,
    };

    pos.x = clamp_world_xz(pos.x, half_xz.x);
    pos.z = clamp_world_xz(pos.z, half_xz.y);

    let instance_id = ObjectId::new_v4();
    let transform = Transform::from_translation(pos);

    let mut entity_commands = if mobility {
        commands.spawn((
            instance_id,
            ObjectPrefabId(saved_root_id),
            Commandable,
            Collider {
                radius: object_radius,
            },
            transform,
            Visibility::Inherited,
        ))
    } else {
        commands.spawn((
            instance_id,
            ObjectPrefabId(saved_root_id),
            BuildObject,
            BuildDimensions { size },
            AabbCollider {
                half_extents: half_xz,
            },
            transform,
            Visibility::Inherited,
        ))
    };
    visuals::spawn_object_visuals(
        &mut entity_commands,
        library,
        asset_server,
        assets,
        meshes,
        materials,
        material_cache,
        mesh_cache,
        saved_root_id,
        None,
    );

    workshop.status = if mobility {
        "Saved prefab to the scene and spawned it next to the hero. Exit Gen3D to select and move it."
            .into()
    } else {
        "Saved prefab to the scene and spawned it to the world. Exit Gen3D to move/rotate/scale it."
            .into()
    };
    workshop.error = None;
    scene_saves.write(SceneSaveRequest::new("Gen3D saved prefab"));

    // Persist a small save artifact for debugging / correlation with agent runs.
    let save_seq = job.bump_save_seq();
    if let Some(run_dir) = job.run_dir_path() {
        let created_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let artifact = serde_json::json!({
            "version": 1,
            "created_at_ms": created_at_ms,
            "save_seq": save_seq,
            "run_id": job.run_id().map(|id| id.to_string()),
            "attempt": job.attempt(),
            "pass": job.pass(),
            "plan_hash": job.plan_hash(),
            "assembly_rev": job.assembly_rev(),
            "workspace_id": job.active_workspace_id(),
            "saved_root_id_uuid": uuid::Uuid::from_u128(saved_root_id).to_string(),
            "instance_id_uuid": uuid::Uuid::from_u128(instance_id.0).to_string(),
            "mobility": mobility,
            "world_pos": [pos.x, pos.y, pos.z],
        });
        let path = run_dir.join(format!("save_{save_seq:04}.json"));
        if let Err(err) = std::fs::write(
            &path,
            serde_json::to_string_pretty(&artifact).unwrap_or_else(|_| artifact.to_string()),
        ) {
            warn!(
                "Gen3D: failed to write save artifact {}: {err}",
                path.display()
            );
        }
    }

    Ok(Gen3dSavedInstance {
        instance_id,
        prefab_id: saved_root_id,
        mobility,
        position: pos,
    })
}

fn save_gen3d_source_bundle_best_effort(bundle_dir: &std::path::Path, draft: &Gen3dDraft) {
    if bundle_dir.exists() {
        if let Err(err) = std::fs::remove_dir_all(&bundle_dir) {
            warn!(
                "Gen3D: failed to clear existing source bundle dir {}: {err}",
                bundle_dir.display()
            );
        }
    }
    if let Err(err) = std::fs::create_dir_all(&bundle_dir) {
        warn!(
            "Gen3D: failed to create source bundle dir {}: {err}",
            bundle_dir.display()
        );
        return;
    }

    if let Err(err) = crate::realm_prefabs::save_prefab_defs_to_dir(
        &bundle_dir,
        super::gen3d_draft_object_id(),
        &draft.defs,
    ) {
        warn!(
            "Gen3D: failed to write source bundle prefabs to {}: {err}",
            bundle_dir.display()
        );
    }
}

fn save_gen3d_edit_bundle_best_effort(
    bundle_path: &std::path::Path,
    job: &Gen3dAiJob,
    saved_root_id: u128,
) {
    let bundle = crate::gen3d::ai::gen3d_build_edit_bundle_v1(job, saved_root_id);
    if let Err(err) = crate::gen3d::ai::gen3d_write_edit_bundle_v1(bundle_path, &bundle) {
        warn!(
            "Gen3D: failed to write edit bundle {}: {err}",
            bundle_path.display()
        );
    }
}

fn save_generated_prefab_descriptor_best_effort(
    prefabs_dir: &std::path::Path,
    root_def: &ObjectDef,
    library: &ObjectLibrary,
    job: &Gen3dAiJob,
    workshop: &Gen3dWorkshop,
) -> crate::prefab_descriptors::PrefabDescriptorFileV1 {
    fn load_optional_json(path: &std::path::Path) -> Option<serde_json::Value> {
        let bytes = std::fs::read(path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn fmt_vec3(v: Vec3) -> String {
        format!("[{:.3},{:.3},{:.3}]", v.x, v.y, v.z)
    }

    fn fmt_vec3_value(value: &serde_json::Value) -> Option<String> {
        let arr = value.as_array()?;
        if arr.len() != 3 {
            return None;
        }
        let x = arr.get(0)?.as_f64()? as f32;
        let y = arr.get(1)?.as_f64()? as f32;
        let z = arr.get(2)?.as_f64()? as f32;
        Some(format!("[{x:.3},{y:.3},{z:.3}]"))
    }

    fn motion_summary_json(library: &ObjectLibrary, root_id: u128) -> serde_json::Value {
        use std::collections::{BTreeMap, BTreeSet, HashSet};

        #[derive(Default)]
        struct Summary {
            slots: u32,
            animated_parts: u32,
            drivers: BTreeSet<String>,
            clip_kinds: BTreeSet<String>,
            loop_duration_min: Option<f32>,
            loop_duration_max: Option<f32>,
            speed_scale_min: Option<f32>,
            speed_scale_max: Option<f32>,
            has_time_offsets: bool,
        }

        fn driver_name(driver: PartAnimationDriver) -> &'static str {
            match driver {
                PartAnimationDriver::Always => "always",
                PartAnimationDriver::MovePhase => "move_phase",
                PartAnimationDriver::MoveDistance => "move_distance",
                PartAnimationDriver::AttackTime => "attack_time",
            }
        }

        fn visit(
            library: &ObjectLibrary,
            object_id: u128,
            visited: &mut HashSet<u128>,
            summaries: &mut BTreeMap<String, Summary>,
        ) {
            if !visited.insert(object_id) {
                return;
            }
            let Some(def) = library.get(object_id) else {
                return;
            };

            for part in def.parts.iter() {
                let mut channels_in_part: BTreeSet<String> = BTreeSet::new();
                for slot in part.animations.iter() {
                    let channel = slot.channel.as_ref().trim();
                    if channel.is_empty() {
                        continue;
                    }
                    channels_in_part.insert(channel.to_string());
                    let entry = summaries.entry(channel.to_string()).or_default();
                    entry.slots = entry.slots.saturating_add(1);
                    entry
                        .drivers
                        .insert(driver_name(slot.spec.driver).to_string());
                    entry.speed_scale_min = Some(
                        entry
                            .speed_scale_min
                            .map_or(slot.spec.speed_scale, |v| v.min(slot.spec.speed_scale)),
                    );
                    entry.speed_scale_max = Some(
                        entry
                            .speed_scale_max
                            .map_or(slot.spec.speed_scale, |v| v.max(slot.spec.speed_scale)),
                    );
                    if slot.spec.time_offset_units.abs() > 1e-6 {
                        entry.has_time_offsets = true;
                    }
                    match &slot.spec.clip {
                        PartAnimationDef::Loop { duration_secs, .. }
                        | PartAnimationDef::Once { duration_secs, .. }
                        | PartAnimationDef::PingPong { duration_secs, .. } => {
                            entry.clip_kinds.insert(
                                match &slot.spec.clip {
                                    PartAnimationDef::Loop { .. } => "loop",
                                    PartAnimationDef::Once { .. } => "once",
                                    PartAnimationDef::PingPong { .. } => "ping_pong",
                                    PartAnimationDef::Spin { .. } => {
                                        unreachable!("spin handled below")
                                    }
                                }
                                .to_string(),
                            );
                            if duration_secs.is_finite() && *duration_secs > 0.0 {
                                entry.loop_duration_min = Some(
                                    entry
                                        .loop_duration_min
                                        .map_or(*duration_secs, |v| v.min(*duration_secs)),
                                );
                                entry.loop_duration_max = Some(
                                    entry
                                        .loop_duration_max
                                        .map_or(*duration_secs, |v| v.max(*duration_secs)),
                                );
                            }
                        }
                        PartAnimationDef::Spin { .. } => {
                            entry.clip_kinds.insert("spin".to_string());
                        }
                    }
                }

                for channel in channels_in_part {
                    if let Some(entry) = summaries.get_mut(&channel) {
                        entry.animated_parts = entry.animated_parts.saturating_add(1);
                    }
                }

                if let ObjectPartKind::ObjectRef { object_id: child } = &part.kind {
                    visit(library, *child, visited, summaries);
                }
            }
        }

        let mut visited: HashSet<u128> = HashSet::new();
        let mut summaries: BTreeMap<String, Summary> = BTreeMap::new();
        visit(library, root_id, &mut visited, &mut summaries);

        let mut channels: Vec<serde_json::Value> = Vec::new();
        for (channel, summary) in summaries {
            let drivers: Vec<String> = summary.drivers.into_iter().collect();
            let clip_kinds: Vec<String> = summary.clip_kinds.into_iter().collect();
            channels.push(serde_json::json!({
                "channel": channel,
                "slots": summary.slots,
                "animated_parts": summary.animated_parts,
                "drivers": drivers,
                "clip_kinds": clip_kinds,
                "loop_duration_secs_min": summary.loop_duration_min,
                "loop_duration_secs_max": summary.loop_duration_max,
                "speed_scale_min": summary.speed_scale_min,
                "speed_scale_max": summary.speed_scale_max,
                "has_time_offsets": summary.has_time_offsets,
            }));
        }

        serde_json::json!({
            "version": 1,
            "channels": channels,
        })
    }

    fn plan_extracted_to_long_text(plan: &serde_json::Value) -> String {
        let mut out = String::new();
        out.push_str("AI plan (extracted):\n");
        if let Some(notes) = plan
            .get("assembly_notes")
            .and_then(|v| v.as_str())
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
        {
            out.push_str(&format!("- assembly_notes: {notes}\n"));
        }
        if let Some(components) = plan.get("components").and_then(|v| v.as_array()) {
            out.push_str("- components:\n");
            for comp in components {
                let name = comp
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if name.is_empty() {
                    continue;
                }
                let purpose = comp
                    .get("purpose")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let modeling = comp
                    .get("modeling_notes")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let size = comp
                    .get("size")
                    .and_then(fmt_vec3_value)
                    .unwrap_or_default();
                if purpose.is_empty() {
                    out.push_str(&format!("  - {name} size≈{size}\n"));
                } else {
                    out.push_str(&format!("  - {name}: {purpose} size≈{size}\n"));
                }
                if !modeling.is_empty() {
                    out.push_str(&format!("    notes: {modeling}\n"));
                }
            }
        }
        out
    }

    fn motion_summary_to_long_text(summary: &serde_json::Value) -> String {
        let mut out = String::new();
        out.push_str("Motions (derived):\n");
        let Some(channels) = summary.get("channels").and_then(|v| v.as_array()) else {
            out.push_str("- <none>\n");
            return out;
        };
        for ch in channels {
            let name = ch
                .get("channel")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if name.is_empty() {
                continue;
            }
            let drivers = ch
                .get("drivers")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let clip_kinds = ch
                .get("clip_kinds")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let parts = ch
                .get("animated_parts")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            out.push_str(&format!(
                "- {name}: drivers={drivers:?} clip_kinds={clip_kinds:?} animated_parts={parts}\n"
            ));
        }
        out
    }

    let created_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u128)
        .unwrap_or(0);

    let prefab_uuid = uuid::Uuid::from_u128(root_def.object_id).to_string();
    let prefab_json = prefabs_dir.join(format!("{prefab_uuid}.json"));
    let descriptor_path =
        crate::prefab_descriptors::prefab_descriptor_path_for_prefab_json(&prefab_json);

    let mut anchors: Vec<crate::prefab_descriptors::PrefabDescriptorAnchorV1> = root_def
        .anchors
        .iter()
        .map(|a| crate::prefab_descriptors::PrefabDescriptorAnchorV1 {
            name: a.name.as_ref().to_string(),
            meaning: None,
            notes: None,
            required: None,
            extra: Default::default(),
        })
        .collect();
    anchors.sort_by(|a, b| a.name.cmp(&b.name));
    anchors.dedup_by(|a, b| a.name == b.name);

    let animation_channels = library.animation_channels_ordered(root_def.object_id);

    let roles = vec![if root_def.mobility.is_some() {
        "unit".to_string()
    } else {
        "building".to_string()
    }];

    let prompt_used = {
        let job_prompt = job.user_prompt_raw().trim();
        if job_prompt.is_empty() {
            workshop.prompt.trim().to_string()
        } else {
            job_prompt.to_string()
        }
    };

    let short = prompt_used
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
        .filter(|v| !v.is_empty());

    let ground_origin_y_m = library.ground_origin_y_or_default(root_def.object_id);
    let mobility_str = root_def.mobility.map(|m| match m.mode {
        MobilityMode::Ground => "ground".to_string(),
        MobilityMode::Air => "air".to_string(),
    });
    let attack_kind_str = root_def.attack.as_ref().map(|a| match a.kind {
        UnitAttackKind::Melee => "melee".to_string(),
        UnitAttackKind::RangedProjectile => "ranged_projectile".to_string(),
    });

    let mut anchor_names: Vec<String> = anchors.iter().map(|a| a.name.clone()).collect();
    anchor_names.sort();
    anchor_names.dedup();

    let motion_summary = motion_summary_json(library, root_def.object_id);

    let mut gen3d_extra: std::collections::BTreeMap<String, serde_json::Value> = Default::default();
    gen3d_extra.insert("attempt".to_string(), serde_json::json!(job.attempt()));
    gen3d_extra.insert("pass".to_string(), serde_json::json!(job.pass()));
    gen3d_extra.insert("plan_hash".to_string(), serde_json::json!(job.plan_hash()));
    gen3d_extra.insert(
        "assembly_rev".to_string(),
        serde_json::json!(job.assembly_rev()),
    );
    gen3d_extra.insert(
        "source_bundle_v1".to_string(),
        serde_json::json!({"dir": "gen3d_source_v1"}),
    );

    let plan_extracted_value = job
        .pass_dir_path()
        .and_then(|dir| load_optional_json(&dir.join("plan_extracted.json")));
    if let Some(plan) = plan_extracted_value.as_ref() {
        gen3d_extra.insert("plan_extracted".to_string(), plan.clone());
    }

    let anchors_for_ai = anchor_names.clone();
    let roles_for_ai = roles.clone();
    let animation_channels_for_ai = animation_channels.clone();
    let animation_channels_for_facts = animation_channels.clone();

    let facts = serde_json::json!({
        "version": 1,
        "size_m": [root_def.size.x, root_def.size.y, root_def.size.z],
        "ground_origin_y_m": ground_origin_y_m,
        "mobility": root_def.mobility.map(|m| serde_json::json!({"mode": match m.mode { MobilityMode::Ground => "ground", MobilityMode::Air => "air" }, "max_speed": m.max_speed})),
        "attack": root_def.attack.as_ref().map(|a| serde_json::json!({"kind": match a.kind { UnitAttackKind::Melee => "melee", UnitAttackKind::RangedProjectile => "ranged_projectile" }, "cooldown_secs": a.cooldown_secs, "damage": a.damage})),
        "anchors": anchor_names,
        "animation_channels": animation_channels_for_facts,
        "label": root_def.label.to_string(),
    });

    let long = {
        let mut out = String::new();
        out.push_str("Prefab facts:\n");
        out.push_str(&format!("- label: {}\n", root_def.label));
        out.push_str(&format!("- roles: {:?}\n", roles));
        out.push_str(&format!("- size_m: {}\n", fmt_vec3(root_def.size)));
        out.push_str(&format!("- ground_origin_y_m: {ground_origin_y_m:.3}\n"));
        out.push_str(&format!(
            "- mobility: {}\n",
            mobility_str.as_deref().unwrap_or("static")
        ));
        out.push_str(&format!(
            "- attack: {}\n",
            attack_kind_str.as_deref().unwrap_or("none")
        ));
        out.push_str(&format!(
            "- anchors: {:?}\n",
            anchors.iter().map(|a| a.name.as_str()).collect::<Vec<_>>()
        ));
        out.push_str(&format!(
            "- animation_channels: {:?}\n\n",
            animation_channels
        ));

        if let Some(plan) = plan_extracted_value.as_ref() {
            out.push_str(&plan_extracted_to_long_text(plan));
            out.push('\n');
        }

        out.push_str(&motion_summary_to_long_text(&motion_summary));
        out
    };

    let mut interfaces_extra: std::collections::BTreeMap<String, serde_json::Value> =
        Default::default();
    interfaces_extra.insert("motion_summary".to_string(), motion_summary.clone());

    let mut top_extra: std::collections::BTreeMap<String, serde_json::Value> = Default::default();
    top_extra.insert("facts".to_string(), facts);

    let descriptor = crate::prefab_descriptors::PrefabDescriptorFileV1 {
        format_version: crate::prefab_descriptors::PREFAB_DESCRIPTOR_FORMAT_VERSION,
        prefab_id: prefab_uuid,
        label: Some(root_def.label.to_string()),
        text: Some(crate::prefab_descriptors::PrefabDescriptorTextV1 {
            short,
            long: Some(long),
        }),
        tags: Vec::new(),
        roles,
        interfaces: Some(crate::prefab_descriptors::PrefabDescriptorInterfacesV1 {
            anchors,
            animation_channels,
            notes: None,
            extra: interfaces_extra,
        }),
        provenance: Some(crate::prefab_descriptors::PrefabDescriptorProvenanceV1 {
            source: Some("gen3d".to_string()),
            created_at_ms: Some(created_at_ms),
            gen3d: Some(crate::prefab_descriptors::PrefabDescriptorGen3dV1 {
                prompt: Some(prompt_used.trim().to_string()).filter(|v| !v.is_empty()),
                style_prompt: None,
                run_id: job.run_id().map(|id| id.to_string()),
                extra: gen3d_extra,
            }),
            revisions: vec![crate::prefab_descriptors::PrefabDescriptorRevisionV1 {
                rev: 1,
                created_at_ms,
                actor: "agent:object".to_string(),
                summary: "generated".to_string(),
                extra: Default::default(),
            }],
            extra: Default::default(),
        }),
        extra: top_extra,
    };

    if let Err(err) =
        crate::prefab_descriptors::save_prefab_descriptor_file(&descriptor_path, &descriptor)
    {
        warn!(
            "Gen3D: failed to write prefab descriptor {}: {err}",
            descriptor_path.display()
        );
    }

    let plan_extracted_text = plan_extracted_value
        .as_ref()
        .and_then(|v| serde_json::to_string_pretty(v).ok());
    super::ai::spawn_prefab_descriptor_meta_enrichment_thread_best_effort(
        job,
        descriptor_path,
        root_def.label.to_string(),
        roles_for_ai,
        root_def.size,
        ground_origin_y_m,
        mobility_str,
        attack_kind_str,
        anchors_for_ai,
        animation_channels_for_ai,
        plan_extracted_text,
        Some(motion_summary),
    );

    descriptor
}
