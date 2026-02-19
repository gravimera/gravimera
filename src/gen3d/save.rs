use bevy::ecs::message::MessageWriter;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use uuid::Uuid;

use crate::assets::SceneAssets;
use crate::constants::{
    BUILD_GRID_SIZE, BUILD_UNIT_SIZE, CROSS_BLOCK_BLOCKING_HEIGHT_FRACTION, WORLD_HALF_SIZE,
};
use crate::geometry::{normalize_flat_direction, snap_to_grid};
use crate::object::registry::{
    ColliderProfile, MeshKey, MobilityMode, MovementBlockRule, ObjectDef, ObjectInteraction,
    ObjectLibrary, ObjectPartKind, PartAnimationDef, PrimitiveParams, PrimitiveVisualDef,
    UnitAttackKind,
};
use crate::object::visuals;
use crate::scene_store::SceneSaveRequest;
use crate::types::{
    AabbCollider, BuildDimensions, BuildObject, Collider, Commandable, ObjectId, ObjectPrefabId,
    Player,
};

use super::ai::Gen3dAiJob;
use super::state::{Gen3dDraft, Gen3dSaveButton, Gen3dWorkshop};

#[derive(SystemParam)]
pub(crate) struct Gen3dSaveRenderWorld<'w> {
    asset_server: Res<'w, AssetServer>,
    assets: Res<'w, SceneAssets>,
    meshes: ResMut<'w, Assets<Mesh>>,
    materials: ResMut<'w, Assets<StandardMaterial>>,
    material_cache: ResMut<'w, visuals::MaterialCache>,
    mesh_cache: ResMut<'w, visuals::PrimitiveMeshCache>,
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

fn saved_root_interaction(collider: ColliderProfile) -> ObjectInteraction {
    match collider {
        ColliderProfile::None => ObjectInteraction::none(),
        _ => ObjectInteraction {
            blocks_bullets: true,
            blocks_laser: true,
            movement_block: Some(MovementBlockRule::UpperBodyFraction(
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
        let (scale, rotation, translation) = composed.to_scale_rotation_translation();
        if !scale.is_finite() || !rotation.is_finite() || !translation.is_finite() {
            return None;
        }
        Some(Transform {
            translation,
            rotation,
            scale,
        })
    }

    fn part_transform_samples(part: &crate::object::registry::ObjectPartDef) -> Vec<Transform> {
        let mut out = Vec::new();
        out.push(part.transform);

        for slot in part.animations.iter() {
            match &slot.spec.clip {
                PartAnimationDef::Loop { keyframes, .. } => {
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
}

pub(super) fn draft_to_saved_defs(draft: &Gen3dDraft) -> Result<(u128, Vec<ObjectDef>), String> {
    let root_id = super::gen3d_draft_object_id();
    let Some(_root) = draft.defs.iter().find(|d| d.object_id == root_id) else {
        return Err("Gen3D: missing root draft object def.".into());
    };

    let defs_map: std::collections::HashMap<u128, ObjectDef> = draft
        .defs
        .iter()
        .map(|d| (d.object_id, d.clone()))
        .collect();
    let mut memo = std::collections::HashMap::<u128, Bounds>::new();
    let mut stack = Vec::new();
    let root_bounds = bounds_of_object(root_id, &defs_map, &mut stack, &mut memo);
    let mut recenter = Vec3::ZERO;
    let mut root_size_override = None;
    if !root_bounds.is_empty() {
        recenter = root_bounds.center();
        let size = root_bounds.size().abs().max(Vec3::splat(0.01));
        root_size_override = Some(size);
    }

    let mut id_map = std::collections::HashMap::<u128, u128>::new();
    for def in &draft.defs {
        let new_id = Uuid::new_v4().as_u128();
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
                saved_root_interaction(new_def.collider)
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

pub(crate) fn gen3d_save_button(
    mode: Res<State<crate::types::GameMode>>,
    active: Res<crate::realm::ActiveRealmScene>,
    mut commands: Commands,
    mut render: Gen3dSaveRenderWorld,
    mut library: ResMut<ObjectLibrary>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut job: ResMut<Gen3dAiJob>,
    draft: Res<Gen3dDraft>,
    player_q: Query<(&Transform, &Collider), With<Player>>,
    mut scene_saves: MessageWriter<SceneSaveRequest>,
    mut last_interaction: Local<Option<Interaction>>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        With<Gen3dSaveButton>,
    >,
) {
    if !matches!(mode.get(), crate::types::GameMode::Gen3D) {
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

            if let Err(err) = gen3d_save_current_draft_from_api(
                &active.realm_id,
                &mut commands,
                &render.asset_server,
                &render.assets,
                &mut *render.meshes,
                &mut *render.materials,
                &mut *render.material_cache,
                &mut *render.mesh_cache,
                &mut library,
                &mut workshop,
                &mut job,
                &draft,
                player_transform,
                player_collider,
                &mut scene_saves,
            ) {
                workshop.error = Some(err);
                workshop.status = "Save failed.".into();
            }
        }
    }

    *last_interaction = Some(*interaction);
}

pub(crate) fn gen3d_save_current_draft_from_api(
    realm_id: &str,
    commands: &mut Commands,
    asset_server: &AssetServer,
    assets: &SceneAssets,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    material_cache: &mut visuals::MaterialCache,
    mesh_cache: &mut visuals::PrimitiveMeshCache,
    library: &mut ObjectLibrary,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &Gen3dDraft,
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
    let (saved_root_id, defs) = draft_to_saved_defs(&snapshot)?;
    crate::realm_prefabs::save_generated_prefab_defs_to_realm(realm_id, saved_root_id, &defs)?;
    for def in defs {
        library.upsert(def);
    }

    let Some(root_def) = library.get(saved_root_id) else {
        return Err("Cannot save: missing saved prefab def.".into());
    };

    save_generated_prefab_descriptor_best_effort(realm_id, root_def, job, workshop);

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

    pos.x = pos
        .x
        .clamp(-WORLD_HALF_SIZE + half_xz.x, WORLD_HALF_SIZE - half_xz.x);
    pos.z = pos
        .z
        .clamp(-WORLD_HALF_SIZE + half_xz.y, WORLD_HALF_SIZE - half_xz.y);

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
        "Saved model as a unit next to the hero. Exit Gen3D to select and move it.".into()
    } else {
        "Saved model to the world. Exit Gen3D to move/rotate/scale it.".into()
    };
    workshop.error = None;
    scene_saves.write(SceneSaveRequest::new("Gen3D saved model"));

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

fn save_generated_prefab_descriptor_best_effort(
    realm_id: &str,
    root_def: &ObjectDef,
    job: &Gen3dAiJob,
    workshop: &Gen3dWorkshop,
) {
    let created_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u128)
        .unwrap_or(0);

    let prefab_uuid = uuid::Uuid::from_u128(root_def.object_id).to_string();
    let prefab_json = crate::realm_prefabs::realm_generated_prefabs_dir(realm_id)
        .join(format!("{prefab_uuid}.json"));
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

    let mut animation_channels: Vec<String> = Vec::new();
    for part in root_def.parts.iter() {
        for slot in part.animations.iter() {
            animation_channels.push(slot.channel.as_ref().to_string());
        }
    }
    animation_channels.sort();
    animation_channels.dedup();

    let roles = vec![if root_def.mobility.is_some() {
        "unit".to_string()
    } else {
        "building".to_string()
    }];

    let short = workshop
        .prompt
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
        .filter(|v| !v.is_empty());

    let descriptor = crate::prefab_descriptors::PrefabDescriptorFileV1 {
        format_version: crate::prefab_descriptors::PREFAB_DESCRIPTOR_FORMAT_VERSION,
        prefab_id: prefab_uuid,
        label: Some(root_def.label.to_string()),
        text: short.map(|short| crate::prefab_descriptors::PrefabDescriptorTextV1 {
            short: Some(short),
            long: None,
        }),
        tags: Vec::new(),
        roles,
        interfaces: Some(crate::prefab_descriptors::PrefabDescriptorInterfacesV1 {
            anchors,
            animation_channels,
            notes: None,
            extra: Default::default(),
        }),
        provenance: Some(crate::prefab_descriptors::PrefabDescriptorProvenanceV1 {
            source: Some("gen3d".to_string()),
            created_at_ms: Some(created_at_ms),
            gen3d: Some(crate::prefab_descriptors::PrefabDescriptorGen3dV1 {
                prompt: Some(workshop.prompt.trim().to_string()).filter(|v| !v.is_empty()),
                style_prompt: None,
                run_id: job.run_id().map(|id| id.to_string()),
                extra: Default::default(),
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
        extra: Default::default(),
    };

    if let Err(err) =
        crate::prefab_descriptors::save_prefab_descriptor_file(&descriptor_path, &descriptor)
    {
        warn!(
            "Gen3D: failed to write prefab descriptor {}: {err}",
            descriptor_path.display()
        );
    }
}
