use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot, ScreenshotCaptured};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::assets::SceneAssets;
use crate::constants::CROSS_BLOCK_BLOCKING_HEIGHT_FRACTION;
use crate::object::registry::{
    ColliderProfile, MeshKey, MobilityMode, MovementBlockRule, ObjectDef, ObjectInteraction,
    ObjectLibrary, ObjectPartKind, PartAnimationDef, PartAnimationDriver, PrimitiveParams,
    PrimitiveVisualDef, UnitAttackKind,
};
use crate::object::visuals;

use super::ai::{Gen3dAiJob, Gen3dDescriptorMetaPolicy};
use super::state::{Gen3dDraft, Gen3dPreview, Gen3dSaveButton, Gen3dWorkshop};
use super::task_queue::Gen3dTaskQueue;

#[derive(SystemParam)]
pub(crate) struct Gen3dSaveRenderWorld<'w> {
    asset_server: Res<'w, AssetServer>,
    assets: Res<'w, SceneAssets>,
    images: ResMut<'w, Assets<Image>>,
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
pub(crate) struct Gen3dSavedPrefab {
    pub(crate) prefab_id: u128,
    pub(crate) mobility: bool,
}

const GEN3D_SAVE_THUMBNAIL_LAYER: usize = 29;
const GEN3D_SAVE_THUMBNAIL_WIDTH_PX: u32 = 256;
const GEN3D_SAVE_THUMBNAIL_HEIGHT_PX: u32 = 256;
const GEN3D_SAVE_THUMBNAIL_TIMEOUT_SECS: u64 = 5;

#[derive(Component)]
struct Gen3dSavedPrefabThumbnailRoot;

#[derive(Component)]
struct Gen3dSavedPrefabThumbnailCamera;

#[derive(Component)]
struct Gen3dSavedPrefabThumbnailLight;

#[derive(Resource, Default)]
pub(crate) struct Gen3dPrefabThumbnailCaptureRuntime {
    active: Option<Gen3dPrefabThumbnailCapture>,
}

#[derive(SystemParam)]
pub(crate) struct Gen3dSaveRuntime<'w> {
    thumbnail_capture: ResMut<'w, Gen3dPrefabThumbnailCaptureRuntime>,
    job: ResMut<'w, Gen3dAiJob>,
}

#[derive(Debug)]
struct Gen3dPrefabThumbnailCaptureProgress {
    expected: usize,
    completed: usize,
}

#[derive(Debug)]
struct Gen3dPrefabThumbnailCapture {
    prefab_id: u128,
    thumbnail_path: PathBuf,
    root: Entity,
    progress: Arc<Mutex<Gen3dPrefabThumbnailCaptureProgress>>,
    started_at: Instant,
    warned_timeout: bool,
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
            // Scene physics/navigation already support standing on build objects when
            // `supports_standing` is enabled. Gen3D outputs frequently include floor tiles,
            // platforms, and roof-walkable structures, so enable standing whenever collision is
            // enabled (and therefore a movement-blocking rule is present).
            supports_standing: collision_enabled,
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

fn bounds_of_object_rest_pose(
    object_id: u128,
    defs: &std::collections::HashMap<u128, ObjectDef>,
    stack: &mut Vec<u128>,
    memo: &mut std::collections::HashMap<u128, Bounds>,
) -> Bounds {
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
            ObjectPartKind::ObjectRef { object_id: child } => {
                let child_bounds = bounds_of_object_rest_pose(*child, defs, stack, memo);
                if child_bounds.is_empty() {
                    continue;
                }

                let child_mat = if let Some(attachment) = part.attachment.as_ref() {
                    let parent_anchor = anchor_transform(def, attachment.parent_anchor.as_ref())
                        .unwrap_or(Transform::IDENTITY);
                    let child_anchor = defs
                        .get(child)
                        .and_then(|child_def| {
                            anchor_transform(child_def, attachment.child_anchor.as_ref())
                        })
                        .unwrap_or(Transform::IDENTITY);

                    parent_anchor.to_matrix()
                        * part.transform.to_matrix()
                        * child_anchor.to_matrix().inverse()
                } else {
                    part.transform.to_matrix()
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
            ObjectPartKind::Model { .. } => {}
        };
    }

    stack.pop();
    memo.insert(object_id, bounds);
    bounds
}

fn prune_nearly_invisible_primitives(def: &mut ObjectDef, alpha_max: f32) -> usize {
    use crate::object::registry::PrimitiveVisualDef;

    let before = def.parts.len();
    def.parts.retain(|part| {
        let ObjectPartKind::Primitive { primitive } = &part.kind else {
            return true;
        };
        let alpha = match primitive {
            PrimitiveVisualDef::Primitive { color, .. } => color.to_srgba().alpha,
            PrimitiveVisualDef::Mesh { .. } => 1.0,
        };
        alpha >= alpha_max
    });
    before.saturating_sub(def.parts.len())
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
                    deform: None,
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
                    deform: None,
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
            family: crate::object::registry::PartAnimationFamily::Base,
            spec: PartAnimationSpec {
                driver: PartAnimationDriver::MovePhase,
                speed_scale: 1.0,
                time_offset_units: 0.0,
                basis: Transform::IDENTITY,
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
    fn draft_to_saved_defs_preserves_root_unit_circle_collider() {
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
                    deform: None,
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
                        deform: None,
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
            draft_to_saved_defs(&draft, false, None, None).expect("save ok");
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
                    deform: None,
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
            draft_to_saved_defs(&draft, false, None, None)
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

        let saved_root_movement_block = draft_to_saved_defs(&draft, true, None, None)
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

    #[test]
    fn draft_to_saved_defs_unit_grounding_ignores_nearly_invisible_root_scaffold() {
        use crate::object::registry::{MobilityDef, ObjectPartDef};

        let root_id = super::super::gen3d_draft_object_id();
        let root_component_id = 0x10u128;
        let foot_id = 0x11u128;

        let foot_def = ObjectDef {
            object_id: foot_id,
            label: "foot".into(),
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
                    deform: None,
                },
                Transform::from_translation(Vec3::new(0.0, -0.1, 0.0))
                    .with_scale(Vec3::new(0.4, 0.2, 0.4)),
            )],
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        };

        let root_component_def = ObjectDef {
            object_id: root_component_id,
            label: "root_component".into(),
            size: Vec3::ONE,
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
                        color: Color::srgba(0.0, 0.0, 0.0, 0.01),
                        unlit: true,
                        deform: None,
                    },
                    Transform::from_scale(Vec3::new(1.0, 2.0, 1.0)),
                ),
                ObjectPartDef::object_ref(foot_id, Transform::IDENTITY),
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
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::CircleXZ { radius: 0.5 },
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: Some(MobilityDef {
                mode: MobilityMode::Ground,
                max_speed: 1.0,
            }),
            anchors: Vec::new(),
            parts: vec![ObjectPartDef::object_ref(
                root_component_id,
                Transform::IDENTITY,
            )],
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        };

        let draft = Gen3dDraft {
            defs: vec![root_def, root_component_def, foot_def],
        };

        let (saved_root_id, saved_defs) =
            draft_to_saved_defs(&draft, false, None, None).expect("save ok");
        let saved_root = saved_defs
            .iter()
            .find(|def| def.object_id == saved_root_id)
            .expect("saved root present");

        let ground_origin_y = saved_root
            .ground_origin_y
            .expect("expected ground_origin_y for unit root");
        assert!(
            (ground_origin_y - 0.1).abs() < 1e-6,
            "ground_origin_y={ground_origin_y}"
        );
    }

    #[test]
    fn draft_to_saved_defs_unit_grounding_prefers_contact_min_y_when_provided() {
        use crate::object::registry::{MobilityDef, ObjectPartDef};

        let root_id = super::super::gen3d_draft_object_id();
        let root_component_id = 0x20u128;
        let foot_id = 0x21u128;

        let foot_def = ObjectDef {
            object_id: foot_id,
            label: "foot".into(),
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
                    deform: None,
                },
                Transform::from_translation(Vec3::new(0.0, -0.1, 0.0))
                    .with_scale(Vec3::new(0.4, 0.2, 0.4)),
            )],
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        };

        let root_component_def = ObjectDef {
            object_id: root_component_id,
            label: "root_component".into(),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: Vec::new(),
            parts: vec![ObjectPartDef::object_ref(foot_id, Transform::IDENTITY)],
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
            collider: ColliderProfile::CircleXZ { radius: 0.5 },
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: Some(MobilityDef {
                mode: MobilityMode::Ground,
                max_speed: 1.0,
            }),
            anchors: Vec::new(),
            parts: vec![ObjectPartDef::object_ref(
                root_component_id,
                Transform::IDENTITY,
            )],
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        };

        let draft = Gen3dDraft {
            defs: vec![root_def, root_component_def, foot_def],
        };

        let min_contact_y = -0.25;
        let (saved_root_id, saved_defs) =
            draft_to_saved_defs(&draft, false, None, Some(min_contact_y)).expect("save ok");
        let saved_root = saved_defs
            .iter()
            .find(|def| def.object_id == saved_root_id)
            .expect("saved root present");

        let ground_origin_y = saved_root
            .ground_origin_y
            .expect("expected ground_origin_y for unit root");
        assert!(
            (ground_origin_y - 0.15).abs() < 1e-6,
            "ground_origin_y={ground_origin_y}"
        );
    }

    #[test]
    fn draft_to_saved_defs_unit_recenters_to_rest_pose_bounds_center_without_expanding_collider() {
        use crate::object::registry::{MobilityDef, ObjectPartDef};

        let root_id = super::super::gen3d_draft_object_id();
        let tail_id = 0x30u128;
        let body_id = 0x31u128;

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
                    deform: None,
                },
                Transform::IDENTITY.with_scale(Vec3::splat(2.0)),
            )],
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        };

        let tail_def = ObjectDef {
            object_id: tail_id,
            label: "tail".into(),
            size: Vec3::ONE,
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
                        color: Color::srgb(1.0, 1.0, 1.0),
                        unlit: false,
                        deform: None,
                    },
                    Transform::IDENTITY,
                ),
                ObjectPartDef::object_ref(
                    body_id,
                    Transform::from_translation(Vec3::new(0.0, 0.0, 10.0)),
                ),
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
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::AabbXZ {
                half_extents: Vec2::splat(0.5),
            },
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: Some(MobilityDef {
                mode: MobilityMode::Ground,
                max_speed: 1.0,
            }),
            anchors: Vec::new(),
            parts: vec![ObjectPartDef::object_ref(tail_id, Transform::IDENTITY)],
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        };

        let draft = Gen3dDraft {
            defs: vec![root_def, tail_def, body_def],
        };

        let (saved_root_id, saved_defs) =
            draft_to_saved_defs(&draft, false, None, None).expect("save ok");
        let saved_root = saved_defs
            .iter()
            .find(|def| def.object_id == saved_root_id)
            .expect("saved root present");

        // Rest-pose bounds: tail cube (-0.5..0.5) and body cube at z=10 with scale 2 => z=9..11.
        // Center z is (min=-0.5 + max=11.0)/2 = 5.25, so the root's part is shifted by -5.25.
        let root_part_z = saved_root.parts[0].transform.translation.z;
        assert!(
            (root_part_z + 5.25).abs() < 1e-3,
            "root_part_z={root_part_z}"
        );

        let ColliderProfile::AabbXZ { half_extents } = saved_root.collider else {
            panic!("expected AabbXZ collider");
        };
        assert!(
            (half_extents.x - 0.5).abs() < 1e-6,
            "half_extents={half_extents:?}"
        );
        assert!(
            (half_extents.y - 0.5).abs() < 1e-6,
            "half_extents={half_extents:?}"
        );
    }
}

pub(super) fn draft_to_saved_defs(
    draft: &Gen3dDraft,
    collision_enabled: bool,
    root_prefab_id_override: Option<u128>,
    min_ground_contact_y_in_root: Option<f32>,
) -> Result<(u128, Vec<ObjectDef>), String> {
    let root_id = super::gen3d_draft_object_id();
    let Some(root_def) = draft.defs.iter().find(|d| d.object_id == root_id) else {
        return Err("Gen3D: missing root draft object def.".into());
    };
    let root_is_unit = root_def.mobility.is_some();
    let root_is_build = !root_is_unit;

    let mut defs_map: std::collections::HashMap<u128, ObjectDef> = draft
        .defs
        .iter()
        .map(|d| (d.object_id, d.clone()))
        .collect();

    if root_is_unit {
        const SCAFFOLD_ALPHA_MAX: f32 = 0.05;

        let root_component_id = root_def.parts.iter().find_map(|part| match &part.kind {
            ObjectPartKind::ObjectRef { object_id } => Some(*object_id),
            _ => None,
        });
        if let Some(root_component_id) = root_component_id {
            if let Some(root_component_def) = defs_map.get_mut(&root_component_id) {
                let has_children = root_component_def
                    .parts
                    .iter()
                    .any(|part| matches!(part.kind, ObjectPartKind::ObjectRef { .. }));
                if has_children {
                    prune_nearly_invisible_primitives(root_component_def, SCAFFOLD_ALPHA_MAX);
                }
            }
        }
    }

    let mut memo = std::collections::HashMap::<u128, Bounds>::new();
    let mut stack = Vec::new();
    let root_bounds = bounds_of_object(root_id, &defs_map, &mut stack, &mut memo);
    let root_size_override =
        (!root_bounds.is_empty()).then(|| root_bounds.size().abs().max(Vec3::splat(0.01)));

    let mut memo_rest = std::collections::HashMap::<u128, Bounds>::new();
    let mut stack_rest = Vec::new();
    let root_bounds_rest =
        bounds_of_object_rest_pose(root_id, &defs_map, &mut stack_rest, &mut memo_rest);

    let mut recenter = Vec3::ZERO;
    let mut root_ground_origin_y = None;
    if root_is_unit {
        if !root_bounds_rest.is_empty() {
            // Units should have a stable pivot near the assembled geometry center (rest pose),
            // even if the plan's component tree is rooted at a non-central link (e.g. tails/chains).
            recenter = root_bounds_rest.center();
        }

        if let Some(min_contact_y) = min_ground_contact_y_in_root.filter(|y| y.is_finite()) {
            root_ground_origin_y = Some((recenter.y - min_contact_y).max(0.0));
        } else if !root_bounds_rest.is_empty() {
            // After applying `recenter`, the new bounds are `root_bounds_rest - recenter`, so:
            // `ground_origin_y = -min.y`.
            root_ground_origin_y = Some((recenter.y - root_bounds_rest.min.y).max(0.0));
        }
    } else if root_is_build && !root_bounds.is_empty() {
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

fn save_gen3d_snapshot_to_scene_and_library(
    realm_id: &str,
    _scene_id: &str,
    library: &mut ObjectLibrary,
    prefab_descriptors: Option<&mut crate::prefab_descriptors::PrefabDescriptorLibrary>,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    snapshot: &Gen3dDraft,
    collision_enabled: bool,
) -> Result<(u128, ObjectDef), String> {
    let (saved_root_id, defs) = draft_to_saved_defs(
        snapshot,
        collision_enabled,
        job.save_overwrite_prefab_id(),
        job.min_ground_contact_y_in_root(),
    )?;
    let prefabs_dir = crate::realm_prefab_packages::save_realm_prefab_package_defs(
        realm_id,
        saved_root_id,
        &defs,
    )?;

    save_gen3d_source_bundle_best_effort(
        &crate::realm_prefab_packages::realm_prefab_package_gen3d_source_dir(
            realm_id,
            saved_root_id,
        ),
        snapshot,
    );
    save_gen3d_edit_bundle_best_effort(
        &crate::realm_prefab_packages::realm_prefab_package_gen3d_edit_bundle_path(
            realm_id,
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
        &prefabs_dir,
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

pub(crate) fn gen3d_save_button(
    env: Gen3dSaveEnv,
    mut commands: Commands,
    mut render: Gen3dSaveRenderWorld,
    mut library: ResMut<ObjectLibrary>,
    mut prefab_descriptors: ResMut<crate::prefab_descriptors::PrefabDescriptorLibrary>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut model_library: ResMut<crate::model_library_ui::ModelLibraryUiState>,
    runtime: Gen3dSaveRuntime,
    draft: Res<Gen3dDraft>,
    preview: Res<Gen3dPreview>,
    mut last_interaction: Local<Option<Interaction>>,
    mut buttons: Query<
        (
            &Interaction,
            &mut BackgroundColor,
            &mut BorderColor,
            &mut Visibility,
            &mut Node,
        ),
        With<Gen3dSaveButton>,
    >,
) {
    if !matches!(env.build_scene.get(), crate::types::BuildScene::Preview) {
        return;
    }

    let running = runtime.job.is_running();

    let Ok((interaction, mut bg, mut border, mut vis, mut node)) = buttons.single_mut() else {
        *last_interaction = None;
        return;
    };

    if !running {
        node.display = Display::None;
        *vis = Visibility::Hidden;
        *bg = BackgroundColor(Color::srgba(0.10, 0.10, 0.11, 0.55));
        *border = BorderColor::all(Color::srgba(0.30, 0.30, 0.34, 0.70));
        *last_interaction = None;
        return;
    }

    node.display = Display::Flex;
    *vis = Visibility::Inherited;

    let enabled = draft.root_def().is_some() && draft.total_non_projectile_primitive_parts() > 0;

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

            let Gen3dSaveRuntime {
                mut thumbnail_capture,
                mut job,
            } = runtime;

            let prev_overwrite = job.save_overwrite_prefab_id();
            let prev_last_saved = job.last_saved_prefab_id();
            if prev_overwrite.is_some() {
                job.set_save_overwrite_prefab_id(None);
            }

            let result = gen3d_save_current_draft_seed_aware_from_api(
                &env.active.realm_id,
                &env.active.scene_id,
                &mut library,
                &mut *prefab_descriptors,
                &mut workshop,
                &mut job,
                &draft,
                preview.show_collision,
            );

            if prev_overwrite.is_some() {
                job.set_save_overwrite_prefab_id(prev_overwrite);
            }
            job.set_last_saved_prefab_id(prev_last_saved);

            match result {
                Ok(saved) => {
                    model_library.mark_models_dirty();

                    let thumbnail_path =
                        crate::realm_prefab_packages::realm_prefab_package_thumbnail_path(
                            &env.active.realm_id,
                            saved.prefab_id,
                        );
                    if let Err(err) = gen3d_request_prefab_thumbnail_capture(
                        &mut commands,
                        &mut *thumbnail_capture,
                        &mut *render.images,
                        &render.asset_server,
                        &render.assets,
                        &mut *render.meshes,
                        &mut *render.materials,
                        &mut *render.material_cache,
                        &mut *render.mesh_cache,
                        &*library,
                        saved.prefab_id,
                        thumbnail_path,
                    ) {
                        warn!("Gen3D: thumbnail capture skipped: {err}");
                    }
                    workshop.status =
                        "Saved snapshot. Open 3D Models to view/spawn it.".to_string();
                }
                Err(err) => {
                    workshop.error = Some(err);
                    workshop.status = "Save snapshot failed.".into();
                }
            }
        }
    }

    *last_interaction = Some(*interaction);
}

pub(crate) fn gen3d_request_prefab_thumbnail_capture(
    commands: &mut Commands,
    runtime: &mut Gen3dPrefabThumbnailCaptureRuntime,
    images: &mut Assets<Image>,
    asset_server: &AssetServer,
    assets: &SceneAssets,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    material_cache: &mut visuals::MaterialCache,
    mesh_cache: &mut visuals::PrimitiveMeshCache,
    library: &ObjectLibrary,
    prefab_id: u128,
    thumbnail_path: PathBuf,
) -> Result<(), String> {
    if let Some(active) = runtime.active.take() {
        cleanup_gen3d_prefab_thumbnail_capture(commands, active);
    }
    let capture = start_gen3d_prefab_thumbnail_capture(
        commands,
        images,
        asset_server,
        assets,
        meshes,
        materials,
        material_cache,
        mesh_cache,
        library,
        prefab_id,
        thumbnail_path,
    )?;
    runtime.active = Some(capture);
    Ok(())
}

fn start_gen3d_prefab_thumbnail_capture(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    asset_server: &AssetServer,
    assets: &SceneAssets,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    material_cache: &mut visuals::MaterialCache,
    mesh_cache: &mut visuals::PrimitiveMeshCache,
    library: &ObjectLibrary,
    prefab_id: u128,
    thumbnail_path: PathBuf,
) -> Result<Gen3dPrefabThumbnailCapture, String> {
    let Some(def) = library.get(prefab_id) else {
        return Err(format!(
            "Prefab {} is not loaded.",
            uuid::Uuid::from_u128(prefab_id)
        ));
    };

    if let Some(parent) = thumbnail_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create thumbnail dir {}: {err}", parent.display()))?;
    }

    let render_layer = bevy::camera::visibility::RenderLayers::layer(GEN3D_SAVE_THUMBNAIL_LAYER);

    let size = def.size.abs().max(Vec3::splat(0.01));
    let origin_y = library.ground_origin_y_or_default(prefab_id);
    let center_y = size.y * 0.5 - origin_y;
    let focus = if center_y.is_finite() {
        Vec3::new(0.0, center_y, 0.0)
    } else {
        Vec3::ZERO
    };

    let root = commands
        .spawn((
            Transform::IDENTITY,
            Visibility::Inherited,
            Gen3dSavedPrefabThumbnailRoot,
        ))
        .id();

    let model_id = {
        let mut entity = commands.spawn((Transform::IDENTITY, Visibility::Inherited));
        crate::object::visuals::spawn_object_visuals_with_settings(
            &mut entity,
            library,
            asset_server,
            assets,
            meshes,
            materials,
            material_cache,
            mesh_cache,
            prefab_id,
            None,
            visuals::VisualSpawnSettings {
                mark_parts: false,
                render_layer: Some(GEN3D_SAVE_THUMBNAIL_LAYER),
            },
        );
        entity.id()
    };
    commands.entity(root).add_child(model_id);

    let lights = [
        (
            Vec3::new(10.0, 18.0, -8.0),
            16_000.0,
            true,
            Color::srgb(1.0, 0.97, 0.94),
        ),
        (
            Vec3::new(-10.0, 10.0, 6.0),
            6_500.0,
            false,
            Color::srgb(0.90, 0.95, 1.0),
        ),
        (
            Vec3::new(0.0, 12.0, -12.0),
            4_000.0,
            false,
            Color::srgb(1.0, 1.0, 1.0),
        ),
        (
            Vec3::new(0.0, -14.0, 0.0),
            4_500.0,
            false,
            Color::srgb(0.96, 0.97, 1.0),
        ),
    ];
    for (offset, illuminance, shadows_enabled, color) in lights {
        let light_id = commands
            .spawn((
                DirectionalLight {
                    shadows_enabled,
                    illuminance,
                    color,
                    ..default()
                },
                Transform::from_translation(focus + offset).looking_at(focus, Vec3::Y),
                render_layer.clone(),
                Gen3dSavedPrefabThumbnailLight,
            ))
            .id();
        commands.entity(root).add_child(light_id);
    }

    let width_px = GEN3D_SAVE_THUMBNAIL_WIDTH_PX;
    let height_px = GEN3D_SAVE_THUMBNAIL_HEIGHT_PX;
    let aspect = width_px.max(1) as f32 / height_px.max(1) as f32;
    let mut projection = bevy::camera::PerspectiveProjection::default();
    projection.aspect_ratio = aspect;
    let fov_y = projection.fov;
    let near = projection.near;

    let yaw = std::f32::consts::FRAC_PI_6;
    let pitch = super::GEN3D_PREVIEW_DEFAULT_PITCH;
    let half_extents = size * 0.5;
    let base_distance = crate::orbit_capture::required_distance_for_view(
        half_extents,
        yaw,
        pitch,
        fov_y,
        aspect,
        near,
    );
    let distance = (base_distance * 1.08).clamp(near + 0.2, 500.0);
    let camera_transform = crate::orbit_capture::orbit_transform(yaw, pitch, distance, focus);

    let target = crate::orbit_capture::create_render_target(images, width_px, height_px);
    let camera = commands
        .spawn((
            Camera3d::default(),
            bevy::camera::Projection::Perspective(projection),
            Camera {
                clear_color: ClearColorConfig::Custom(Color::srgb(0.93, 0.94, 0.96)),
                ..default()
            },
            RenderTarget::Image(target.clone().into()),
            Tonemapping::TonyMcMapface,
            render_layer.clone(),
            camera_transform,
            Gen3dSavedPrefabThumbnailCamera,
        ))
        .id();
    commands.entity(root).add_child(camera);

    let progress = Arc::new(Mutex::new(Gen3dPrefabThumbnailCaptureProgress {
        expected: 1,
        completed: 0,
    }));
    let path_for_capture = thumbnail_path.clone();
    let progress_for_capture = progress.clone();
    let _screenshot = commands
        .spawn(Screenshot::image(target))
        .observe(move |event: On<ScreenshotCaptured>| {
            let mut saver = save_to_disk(path_for_capture.clone());
            saver(event);
            if let Ok(mut guard) = progress_for_capture.lock() {
                guard.completed = guard.completed.saturating_add(1);
            }
        })
        .id();

    Ok(Gen3dPrefabThumbnailCapture {
        prefab_id,
        thumbnail_path,
        root,
        progress,
        started_at: Instant::now(),
        warned_timeout: false,
    })
}

fn cleanup_gen3d_prefab_thumbnail_capture(
    commands: &mut Commands,
    capture: Gen3dPrefabThumbnailCapture,
) {
    commands.entity(capture.root).try_despawn();
}

pub(crate) fn gen3d_prefab_thumbnail_capture_poll(
    mut commands: Commands,
    mut runtime: ResMut<Gen3dPrefabThumbnailCaptureRuntime>,
    mut model_library: Option<ResMut<crate::model_library_ui::ModelLibraryUiState>>,
) {
    let done = {
        let Some(capture) = runtime.active.as_mut() else {
            return;
        };

        let done = match capture.progress.lock() {
            Ok(guard) => guard.completed >= guard.expected.max(1),
            Err(_) => true,
        };

        if !done
            && capture.started_at.elapsed() > Duration::from_secs(GEN3D_SAVE_THUMBNAIL_TIMEOUT_SECS)
            && !capture.warned_timeout
        {
            capture.warned_timeout = true;
            warn!(
                "Gen3D: thumbnail capture is taking longer than {}s.",
                GEN3D_SAVE_THUMBNAIL_TIMEOUT_SECS
            );
        }

        done
    };

    if !done {
        return;
    }

    let Some(capture) = runtime.active.take() else {
        return;
    };

    let thumbnail_exists = std::fs::metadata(&capture.thumbnail_path).is_ok();
    if !thumbnail_exists {
        debug!(
            "Gen3D: thumbnail capture finished but output is missing (prefab={}): {}",
            Uuid::from_u128(capture.prefab_id),
            capture.thumbnail_path.display()
        );
    }
    if let Some(state) = model_library.as_mut() {
        state.mark_models_dirty();
    }

    cleanup_gen3d_prefab_thumbnail_capture(&mut commands, capture);
}

pub(crate) fn gen3d_auto_save_when_done(
    env: Gen3dSaveEnv,
    mut commands: Commands,
    mut render: Gen3dSaveRenderWorld,
    mut library: ResMut<ObjectLibrary>,
    mut prefab_descriptors: ResMut<crate::prefab_descriptors::PrefabDescriptorLibrary>,
    mut task_queue: ResMut<Gen3dTaskQueue>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut model_library: ResMut<crate::model_library_ui::ModelLibraryUiState>,
    runtime: Gen3dSaveRuntime,
    mut draft: ResMut<Gen3dDraft>,
    preview: Res<Gen3dPreview>,
    mut last_handled_run: Local<Option<Uuid>>,
) {
    struct SwapGuard {
        workshop_a: *mut Gen3dWorkshop,
        job_a: *mut Gen3dAiJob,
        draft_a: *mut Gen3dDraft,
        workshop_b: *mut Gen3dWorkshop,
        job_b: *mut Gen3dAiJob,
        draft_b: *mut Gen3dDraft,
    }

    impl SwapGuard {
        unsafe fn new(
            workshop_a: &mut Gen3dWorkshop,
            job_a: &mut Gen3dAiJob,
            draft_a: &mut Gen3dDraft,
            workshop_b: &mut Gen3dWorkshop,
            job_b: &mut Gen3dAiJob,
            draft_b: &mut Gen3dDraft,
        ) -> Self {
            std::mem::swap(workshop_a, workshop_b);
            std::mem::swap(job_a, job_b);
            std::mem::swap(draft_a, draft_b);
            Self {
                workshop_a: workshop_a as *mut Gen3dWorkshop,
                job_a: job_a as *mut Gen3dAiJob,
                draft_a: draft_a as *mut Gen3dDraft,
                workshop_b: workshop_b as *mut Gen3dWorkshop,
                job_b: job_b as *mut Gen3dAiJob,
                draft_b: draft_b as *mut Gen3dDraft,
            }
        }
    }

    impl Drop for SwapGuard {
        fn drop(&mut self) {
            unsafe {
                std::mem::swap(&mut *self.workshop_a, &mut *self.workshop_b);
                std::mem::swap(&mut *self.job_a, &mut *self.job_b);
                std::mem::swap(&mut *self.draft_a, &mut *self.draft_b);
            }
        }
    }

    let Gen3dSaveRuntime {
        mut thumbnail_capture,
        mut job,
    } = runtime;

    let _swap_guard = task_queue
        .running_session_id
        .and_then(|running_id| {
            (running_id != task_queue.active_session_id)
                .then_some(running_id)
                .and_then(|id| task_queue.inactive_states.get_mut(&id))
        })
        .map(|state| unsafe {
            SwapGuard::new(
                &mut *workshop,
                &mut *job,
                &mut *draft,
                &mut state.workshop,
                &mut state.job,
                &mut state.draft,
            )
        });

    let Some(run_id) = job.run_id() else {
        return;
    };
    if job.is_running() || !job.is_build_complete() {
        return;
    }

    if *last_handled_run == Some(run_id) {
        return;
    }
    *last_handled_run = Some(run_id);

    let components = draft.component_count();
    let parts = draft.total_primitive_parts();
    let motions = preview.animation_channels.len();
    let attempt = job.attempt() + 1;
    let step = job.step() + 1;

    let run_time = job
        .run_elapsed()
        .map(|d| {
            let secs = d.as_secs();
            if secs < 60 {
                format!("{:.1}s", d.as_secs_f32())
            } else {
                format!("{}m {}s", secs / 60, secs % 60)
            }
        })
        .unwrap_or_else(|| "—".into());

    // Close any dangling "active step" so summary entries don't implicitly mark it as interrupted.
    workshop
        .status_log
        .finish_step_if_active("Run finished.".to_string());

    fn short_uuid(prefab_id: u128) -> String {
        let uuid = Uuid::from_u128(prefab_id).to_string();
        uuid.chars().take(8).collect()
    }

    fn summarize_error(err: &str) -> String {
        let first_line = err.trim().lines().next().unwrap_or("").trim();
        if first_line.is_empty() {
            return "error".to_string();
        }
        const MAX: usize = 72;
        let mut out: String = first_line.chars().take(MAX).collect();
        if first_line.chars().count() > MAX {
            out.push('…');
        }
        out
    }

    let mut save_note = "skipped".to_string();
    let session_id_for_meta = task_queue
        .running_session_id
        .unwrap_or(task_queue.active_session_id);

    let wants_auto_save = draft.root_def().is_some()
        && draft.total_non_projectile_primitive_parts() > 0
        && job.last_saved_prefab_id().is_none();

    if wants_auto_save {
        workshop.status_log.start_step(
            "Auto-save".to_string(),
            "Build completed; save a prefab and refresh the Prefab tab.".to_string(),
        );

        if job.overwrite_save_blocked_by_qa_errors() {
            save_note = "skipped (qa errors)".to_string();
            workshop.status_log.finish_step_if_active(format!(
                "Skipped: QA errors block overwrite save (validate_ok={:?} smoke_ok={:?} motion_ok={:?}).",
                job.last_validate_ok(),
                job.last_smoke_ok(),
                job.last_motion_ok()
            ));
        } else {
            match gen3d_save_current_draft_seed_aware_from_api(
                &env.active.realm_id,
                &env.active.scene_id,
                &mut library,
                &mut *prefab_descriptors,
                &mut workshop,
                &mut job,
                &draft,
                preview.show_collision,
            ) {
                Ok(saved) => {
                    model_library.mark_models_dirty();

                    let thumbnail_path =
                        crate::realm_prefab_packages::realm_prefab_package_thumbnail_path(
                            &env.active.realm_id,
                            saved.prefab_id,
                        );
                    if let Err(err) = gen3d_request_prefab_thumbnail_capture(
                        &mut commands,
                        &mut *thumbnail_capture,
                        &mut *render.images,
                        &render.asset_server,
                        &render.assets,
                        &mut *render.meshes,
                        &mut *render.materials,
                        &mut *render.material_cache,
                        &mut *render.mesh_cache,
                        &*library,
                        saved.prefab_id,
                        thumbnail_path,
                    ) {
                        warn!("Gen3D: thumbnail capture skipped: {err}");
                    }

                    if job.edit_base_prefab_id().is_none() {
                        job.promote_to_edit_overwrite_from_descriptor(
                            saved.prefab_id,
                            prefab_descriptors.get(saved.prefab_id),
                        );
                        if let Some(meta) = task_queue.metas.get_mut(&session_id_for_meta) {
                            if matches!(meta.kind, super::task_queue::Gen3dSessionKind::NewBuild) {
                                meta.kind = super::task_queue::Gen3dSessionKind::EditOverwrite {
                                    prefab_id: saved.prefab_id,
                                };
                            }
                        }
                        workshop.status = "Build finished. Prefab saved. Click Edit to start a new run; auto-save overwrites the same prefab id.".into();
                    }

                    let short = short_uuid(saved.prefab_id);
                    save_note = format!("ok ({short})");
                    workshop
                        .status_log
                        .finish_step_if_active(format!("OK ({short})"));
                }
                Err(err) => {
                    let err = summarize_error(err.as_str());
                    save_note = format!("failed ({err})");
                    workshop
                        .status_log
                        .finish_step_if_active(format!("Error: {err}"));
                }
            }
        }
    } else if job.last_saved_prefab_id().is_some() {
        save_note = "skipped (already saved)".to_string();
    } else if draft.root_def().is_none() || draft.total_non_projectile_primitive_parts() == 0 {
        save_note = "skipped (empty draft)".to_string();
    }

    let summary = format!(
        "Done | comps={components} parts={parts} motion={motions} | attempt={attempt} step={step} | time={run_time} | auto-save={save_note}"
    );
    workshop.status_log.start_step(
        "Build summary".to_string(),
        "Final counters for this run.".to_string(),
    );
    workshop.status_log.finish_step(summary);
}

pub(crate) fn gen3d_save_current_draft_seed_aware_from_api(
    realm_id: &str,
    scene_id: &str,
    library: &mut ObjectLibrary,
    prefab_descriptors: &mut crate::prefab_descriptors::PrefabDescriptorLibrary,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &Gen3dDraft,
    collision_enabled: bool,
) -> Result<Gen3dSavedPrefab, String> {
    if draft.root_def().is_none() || draft.total_non_projectile_primitive_parts() == 0 {
        return Err("Cannot save: draft is empty.".into());
    }

    let overwrite_prefab_id = job.save_overwrite_prefab_id();
    let base_prefab_id = job.edit_base_prefab_id();

    // Snapshot the draft at call time so a concurrent Build run can't mutate it mid-save.
    let snapshot = Gen3dDraft {
        defs: draft.defs.clone(),
    };
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

    workshop.status = match (overwrite_prefab_id, base_prefab_id) {
        (Some(_), _) => {
            "Saved prefab (overwrite). Exit Gen3D and open 3D Models to view/spawn it.".into()
        }
        (None, Some(_)) => {
            "Saved prefab (fork). Exit Gen3D and open 3D Models to view/spawn it.".into()
        }
        (None, None) => "Saved prefab. Exit Gen3D and open 3D Models to view/spawn it.".into(),
    };
    workshop.error = None;

    job.set_last_saved_prefab_id(Some(saved_root_id));

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
            "kind": if overwrite_prefab_id.is_some() { "edit_overwrite" } else if base_prefab_id.is_some() { "fork" } else { "new" },
            "run_id": job.run_id().map(|id| id.to_string()),
            "attempt": job.attempt(),
            "step": job.step(),
            "plan_hash": job.plan_hash(),
            "assembly_rev": job.assembly_rev(),
            "workspace_id": job.active_workspace_id(),
            "base_prefab_id_uuid": base_prefab_id.map(|id| uuid::Uuid::from_u128(id).to_string()),
            "saved_root_id_uuid": uuid::Uuid::from_u128(saved_root_id).to_string(),
            "mobility": root_def.mobility.is_some(),
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

    Ok(Gen3dSavedPrefab {
        prefab_id: saved_root_id,
        mobility: root_def.mobility.is_some(),
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
                PartAnimationDriver::ActionTime => "action_time",
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

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u128)
        .unwrap_or(0);

    let prefab_uuid = uuid::Uuid::from_u128(root_def.object_id).to_string();
    let prefab_json = prefabs_dir.join(format!("{prefab_uuid}.json"));
    let descriptor_path =
        crate::prefab_descriptors::prefab_descriptor_path_for_prefab_json(&prefab_json);

    let existing_descriptor: Option<crate::prefab_descriptors::PrefabDescriptorFileV1> =
        std::fs::read(&descriptor_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|json| serde_json::from_value(json).ok());

    fn sum_revision_input_tokens(
        revisions: &[crate::prefab_descriptors::PrefabDescriptorRevisionV1],
    ) -> u64 {
        let mut out: u64 = 0;
        for rev in revisions {
            if let Some(tokens) = rev.extra.get("tokens_input").and_then(|v| v.as_u64()) {
                out = out.saturating_add(tokens);
            }
        }
        out
    }

    fn sum_revision_output_tokens(
        revisions: &[crate::prefab_descriptors::PrefabDescriptorRevisionV1],
    ) -> u64 {
        let mut out: u64 = 0;
        for rev in revisions {
            if let Some(tokens) = rev.extra.get("tokens_output").and_then(|v| v.as_u64()) {
                out = out.saturating_add(tokens);
            }
        }
        out
    }

    let run_input_tokens = job.current_run_input_tokens();
    let run_output_tokens = job.current_run_output_tokens();
    let run_unsplit_tokens = job.current_run_unsplit_tokens();
    let run_duration_ms = job.run_elapsed().map(|d| d.as_millis() as u128);

    let prev_total_input_tokens = existing_descriptor
        .as_ref()
        .and_then(|d| d.provenance.as_ref())
        .and_then(|p| p.total_input_tokens)
        .or_else(|| {
            existing_descriptor
                .as_ref()
                .and_then(|d| d.provenance.as_ref())
                .map(|p| sum_revision_input_tokens(&p.revisions))
        })
        .unwrap_or(0);
    let prev_total_output_tokens = existing_descriptor
        .as_ref()
        .and_then(|d| d.provenance.as_ref())
        .and_then(|p| p.total_output_tokens)
        .or_else(|| {
            existing_descriptor
                .as_ref()
                .and_then(|d| d.provenance.as_ref())
                .map(|p| sum_revision_output_tokens(&p.revisions))
        })
        .unwrap_or(0);
    let prev_total_unsplit_tokens = existing_descriptor
        .as_ref()
        .and_then(|d| d.provenance.as_ref())
        .and_then(|p| p.total_unsplit_tokens)
        .or_else(|| {
            existing_descriptor
                .as_ref()
                .and_then(|d| d.provenance.as_ref())
                .map(|p| {
                    let mut out: u64 = 0;
                    for rev in &p.revisions {
                        if let Some(tokens) =
                            rev.extra.get("tokens_unsplit").and_then(|v| v.as_u64())
                        {
                            out = out.saturating_add(tokens);
                        }
                    }
                    out
                })
        })
        .unwrap_or(0);
    let new_total_input_tokens = prev_total_input_tokens.saturating_add(run_input_tokens);
    let new_total_output_tokens = prev_total_output_tokens.saturating_add(run_output_tokens);
    let new_total_unsplit_tokens = prev_total_unsplit_tokens.saturating_add(run_unsplit_tokens);
    let created_at_ms = existing_descriptor
        .as_ref()
        .and_then(|d| d.provenance.as_ref())
        .and_then(|p| p.created_at_ms)
        .unwrap_or(now_ms);
    let created_duration_ms = existing_descriptor
        .as_ref()
        .and_then(|d| d.provenance.as_ref())
        .and_then(|p| p.created_duration_ms)
        .or_else(|| {
            existing_descriptor
                .is_none()
                .then_some(run_duration_ms)
                .flatten()
        });

    let mut revisions = existing_descriptor
        .as_ref()
        .and_then(|d| d.provenance.as_ref())
        .map(|p| p.revisions.clone())
        .unwrap_or_default();
    let next_rev = revisions
        .iter()
        .map(|rev| rev.rev)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let revision_summary = if existing_descriptor.is_some() {
        "saved"
    } else {
        "generated"
    };

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

    let mut revision_extra: std::collections::BTreeMap<String, serde_json::Value> =
        Default::default();
    if !prompt_used.trim().is_empty() {
        revision_extra.insert(
            "prompt".to_string(),
            serde_json::Value::String(prompt_used.trim().to_string()),
        );
    }
    revision_extra.insert(
        "tokens_input".to_string(),
        serde_json::Value::from(run_input_tokens),
    );
    revision_extra.insert(
        "tokens_output".to_string(),
        serde_json::Value::from(run_output_tokens),
    );
    revision_extra.insert(
        "tokens_unsplit".to_string(),
        serde_json::Value::from(run_unsplit_tokens),
    );
    revision_extra.insert(
        "tokens_total".to_string(),
        serde_json::Value::from(
            run_input_tokens
                .saturating_add(run_output_tokens)
                .saturating_add(run_unsplit_tokens),
        ),
    );
    if let Some(ms) = run_duration_ms {
        let ms = ms.min(u128::from(u64::MAX)) as u64;
        revision_extra.insert("duration_ms".to_string(), serde_json::Value::from(ms));
    }
    if let Some((policy, meta)) = job.descriptor_meta_for_save() {
        revision_extra.insert(
            "descriptor_meta_policy".to_string(),
            serde_json::Value::String(match policy {
                Gen3dDescriptorMetaPolicy::Suggest => "suggest".to_string(),
                Gen3dDescriptorMetaPolicy::Preserve => "preserve".to_string(),
            }),
        );
        revision_extra.insert(
            "descriptor_meta_v1".to_string(),
            serde_json::json!({
                "version": meta.version,
                "name": meta.name.trim(),
                "short": meta.short.trim(),
                "tags": meta.tags.clone(),
            }),
        );
    }

    revisions.push(crate::prefab_descriptors::PrefabDescriptorRevisionV1 {
        rev: next_rev,
        created_at_ms: now_ms,
        actor: "agent:object".to_string(),
        summary: revision_summary.to_string(),
        extra: revision_extra,
    });

    let short = prompt_used
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
        .filter(|v| !v.is_empty());

    fn clamp_words(text: &str, max_words: usize) -> Option<String> {
        let words: Vec<&str> = text
            .trim()
            .split_whitespace()
            .filter(|w| !w.trim().is_empty())
            .take(max_words)
            .collect();
        (!words.is_empty()).then_some(words.join(" "))
    }

    let name = clamp_words(root_def.label.as_ref(), 3).or_else(|| {
        prompt_used
            .lines()
            .find(|line| !line.trim().is_empty())
            .and_then(|line| clamp_words(line, 3))
    });

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
    gen3d_extra.insert("step".to_string(), serde_json::json!(job.step()));
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
        .step_dir_path()
        .and_then(|dir| load_optional_json(&dir.join("plan_extracted.json")));
    if let Some(plan) = plan_extracted_value.as_ref() {
        gen3d_extra.insert("plan_extracted".to_string(), plan.clone());
    }

    if let Some((policy, meta)) = job.descriptor_meta_for_save() {
        gen3d_extra.insert(
            "descriptor_meta_policy".to_string(),
            serde_json::Value::String(match policy {
                Gen3dDescriptorMetaPolicy::Suggest => "suggest".to_string(),
                Gen3dDescriptorMetaPolicy::Preserve => "preserve".to_string(),
            }),
        );
        gen3d_extra.insert(
            "descriptor_meta_v1".to_string(),
            serde_json::json!({
                "version": meta.version,
                "name": meta.name.trim(),
                "short": meta.short.trim(),
                "tags": meta.tags.clone(),
            }),
        );
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

    let mut descriptor = crate::prefab_descriptors::PrefabDescriptorFileV1 {
        format_version: crate::prefab_descriptors::PREFAB_DESCRIPTOR_FORMAT_VERSION,
        prefab_id: prefab_uuid,
        label: name,
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
            created_duration_ms,
            modified_at_ms: Some(now_ms),
            total_input_tokens: Some(new_total_input_tokens),
            total_output_tokens: Some(new_total_output_tokens),
            total_unsplit_tokens: Some(new_total_unsplit_tokens),
            gen3d: Some(crate::prefab_descriptors::PrefabDescriptorGen3dV1 {
                prompt: Some(prompt_used.trim().to_string()).filter(|v| !v.is_empty()),
                style_prompt: None,
                run_id: job.run_id().map(|id| id.to_string()),
                extra: gen3d_extra,
            }),
            revisions,
            extra: Default::default(),
        }),
        extra: top_extra,
    };

    if let Some((policy, meta)) = job.descriptor_meta_for_save() {
        match policy {
            Gen3dDescriptorMetaPolicy::Suggest => {
                let mut should_update_label = true;
                if let Some(label) = descriptor
                    .label
                    .as_ref()
                    .map(|v| v.trim())
                    .filter(|v| !v.is_empty())
                {
                    should_update_label = false;
                    if let Some(clamped) = clamp_words(root_def.label.as_ref(), 3) {
                        if label == clamped {
                            should_update_label = true;
                        }
                    }
                    if label.eq_ignore_ascii_case(root_def.label.as_ref().trim()) {
                        should_update_label = true;
                    }
                }

                if should_update_label && !meta.name.trim().is_empty() {
                    descriptor.label = Some(meta.name.trim().to_string());
                }

                let mut should_update_short = true;
                if let Some(text) = descriptor.text.as_ref().and_then(|t| t.short.as_deref()) {
                    if !text.trim().is_empty() {
                        should_update_short = false;
                        if let Some(first_line) = prompt_used.lines().find(|l| !l.trim().is_empty())
                        {
                            if text.trim() == first_line.trim() {
                                should_update_short = true;
                            }
                        }
                    }
                }

                if should_update_short && !meta.short.trim().is_empty() {
                    let text = descriptor.text.get_or_insert_with(Default::default);
                    text.short = Some(meta.short.trim().to_string());
                }
                descriptor.tags = meta.tags.clone();
            }
            Gen3dDescriptorMetaPolicy::Preserve => {
                if meta.name.trim().is_empty() {
                    descriptor.label = None;
                } else {
                    descriptor.label = Some(meta.name.trim().to_string());
                }

                if meta.short.trim().is_empty() {
                    if let Some(text) = descriptor.text.as_mut() {
                        text.short = None;
                    }
                } else {
                    let text = descriptor.text.get_or_insert_with(Default::default);
                    text.short = Some(meta.short.trim().to_string());
                }
                descriptor.tags = meta.tags.clone();
            }
        }
    }

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
    if job.descriptor_meta_for_save().is_none() {
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
    }

    descriptor
}
