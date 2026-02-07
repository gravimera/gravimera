use bevy::ecs::message::MessageWriter;
use bevy::prelude::*;
use uuid::Uuid;

use crate::assets::SceneAssets;
use crate::constants::{BUILD_GRID_SIZE, CROSS_BLOCK_BLOCKING_HEIGHT_FRACTION, WORLD_HALF_SIZE};
use crate::geometry::{normalize_flat_direction, snap_to_grid};
use crate::object::registry::{
    ColliderProfile, MeshKey, MovementBlockRule, ObjectDef, ObjectInteraction, ObjectLibrary,
    ObjectPartKind, PrimitiveParams, PrimitiveVisualDef, UnitAttackKind,
};
use crate::object::visuals;
use crate::scene_store::SceneSaveRequest;
use crate::types::{
    AabbCollider, BuildDimensions, BuildObject, Collider, Commandable, ObjectId, ObjectPrefabId,
    Player,
};

use super::ai::Gen3dAiJob;
use super::state::{Gen3dDraft, Gen3dSaveButton, Gen3dWorkshop};

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

                let abs = Mat3::from_quat(part.transform.rotation).abs();
                let ext = abs * local_half;
                let center = part.transform.translation;
                bounds.include_point(center - ext);
                bounds.include_point(center + ext);
            }
            ObjectPartKind::ObjectRef { object_id: child } => {
                let child_bounds = bounds_of_object(*child, defs, stack, memo);
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
        }
    }

    stack.pop();
    memo.insert(object_id, bounds);
    bounds
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::registry::{AnchorDef, AttachmentDef, ObjectPartDef};

    #[test]
    fn bounds_of_object_respects_attachment_anchors() {
        let parent_id = 1u128;
        let child_id = 2u128;

        let child_def = ObjectDef {
            object_id: child_id,
            label: "child".into(),
            size: Vec3::ONE,
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
}

fn draft_to_saved_defs(draft: &Gen3dDraft) -> Result<(u128, Vec<ObjectDef>), String> {
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
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    assets: Res<SceneAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut material_cache: ResMut<visuals::MaterialCache>,
    mut mesh_cache: ResMut<visuals::PrimitiveMeshCache>,
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
                &mut commands,
                &asset_server,
                &assets,
                &mut meshes,
                &mut materials,
                &mut material_cache,
                &mut mesh_cache,
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
    for def in defs {
        library.upsert(def);
    }

    let Some(root_def) = library.get(saved_root_id) else {
        return Err("Cannot save: missing saved prefab def.".into());
    };

    let size = root_def.size;
    let half_xz = collider_half_xz(root_def.collider, size);
    let object_radius = half_xz.x.max(half_xz.y).max(0.1);
    let mobility = root_def.mobility.is_some();

    let forward = normalize_flat_direction(player_transform.rotation * Vec3::Z).unwrap_or(Vec3::Z);
    let right = Vec3::Y.cross(forward).normalize_or_zero();
    let distance = player_collider.radius + object_radius + BUILD_GRID_SIZE;

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
    let spacing = (object_radius * 2.0 + BUILD_GRID_SIZE * 2.0).max(BUILD_GRID_SIZE * 4.0);
    let radial = distance + ring as f32 * spacing;

    let mut pos = player_transform.translation + dir * radial;
    pos.x = snap_to_grid(pos.x, BUILD_GRID_SIZE);
    pos.z = snap_to_grid(pos.z, BUILD_GRID_SIZE);
    pos.y = size.y.max(0.01) * 0.5;

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
