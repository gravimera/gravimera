use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use crate::assets::SceneAssets;
use crate::object::registry::{
    AttachmentDef, MaterialKey, MeshKey, ObjectLibrary, ObjectPartKind, PartAnimationDef,
    PartAnimationDriver, PartAnimationSlot, PartAnimationSpec, PrimitiveParams, PrimitiveVisualDef,
    UnitAttackKind,
};
use crate::types::{
    AnimationChannelsActive, AttackClock, ForcedAnimationChannel, LocomotionClock, ObjectPrefabId,
};

const MAX_VISUAL_DEPTH: usize = 32;

#[derive(Resource, Default)]
pub(crate) struct MaterialCache {
    map: HashMap<MaterialCacheKey, Handle<StandardMaterial>>,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum MaterialCacheKey {
    Color {
        rgba: [u8; 4],
        unlit: bool,
    },
    Tinted {
        base: AssetId<StandardMaterial>,
        tint_rgba: [u8; 4],
    },
}

impl MaterialCache {
    fn get_or_create_color(
        &mut self,
        materials: &mut Assets<StandardMaterial>,
        color: Color,
        unlit: bool,
    ) -> Handle<StandardMaterial> {
        let rgba = to_rgba8(color);
        let key = MaterialCacheKey::Color { rgba, unlit };
        if let Some(existing) = self.map.get(&key) {
            return existing.clone();
        }

        let base_color = Color::srgba(
            rgba[0] as f32 / 255.0,
            rgba[1] as f32 / 255.0,
            rgba[2] as f32 / 255.0,
            rgba[3] as f32 / 255.0,
        );
        let alpha_mode = if rgba[3] < 255 {
            AlphaMode::Blend
        } else {
            AlphaMode::Opaque
        };

        let handle = materials.add(StandardMaterial {
            base_color,
            unlit,
            alpha_mode,
            metallic: 0.0,
            perceptual_roughness: 0.92,
            ..default()
        });
        self.map.insert(key, handle.clone());
        handle
    }

    fn get_or_create_tinted(
        &mut self,
        materials: &mut Assets<StandardMaterial>,
        base: &Handle<StandardMaterial>,
        tint: Color,
    ) -> Handle<StandardMaterial> {
        let tint_rgba = to_rgba8(tint);
        if tint_rgba == [255, 255, 255, 255] {
            return base.clone();
        }

        let key = MaterialCacheKey::Tinted {
            base: base.id(),
            tint_rgba,
        };
        if let Some(existing) = self.map.get(&key) {
            return existing.clone();
        }

        let Some(base_material) = materials.get(base) else {
            return base.clone();
        };

        let mut material = base_material.clone();
        material.base_color = multiply_color(material.base_color, tint);
        if matches!(material.alpha_mode, AlphaMode::Opaque) && tint_rgba[3] < 255 {
            material.alpha_mode = AlphaMode::Blend;
        }

        let handle = materials.add(material);
        self.map.insert(key, handle.clone());
        handle
    }
}

#[derive(Resource, Default)]
pub(crate) struct PrimitiveMeshCache {
    map: HashMap<PrimitiveMeshCacheKey, Handle<Mesh>>,
    mirrored_winding: HashMap<AssetId<Mesh>, Handle<Mesh>>,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum PrimitiveMeshCacheKey {
    Capsule {
        radius_milli: i32,
        half_length_milli: i32,
    },
    ConicalFrustum {
        radius_top_milli: i32,
        radius_bottom_milli: i32,
        height_milli: i32,
    },
    Torus {
        minor_radius_milli: i32,
        major_radius_milli: i32,
    },
}

fn quantize_milli(v: f32) -> i32 {
    if !v.is_finite() {
        return 0;
    }
    (v * 1000.0).round().clamp(i32::MIN as f32, i32::MAX as f32) as i32
}

impl PrimitiveMeshCache {
    pub(crate) fn get_or_create(
        &mut self,
        meshes: &mut Assets<Mesh>,
        params: PrimitiveParams,
    ) -> Handle<Mesh> {
        let key = match params {
            PrimitiveParams::Capsule {
                radius,
                half_length,
            } => PrimitiveMeshCacheKey::Capsule {
                radius_milli: quantize_milli(radius),
                half_length_milli: quantize_milli(half_length),
            },
            PrimitiveParams::ConicalFrustum {
                radius_top,
                radius_bottom,
                height,
            } => PrimitiveMeshCacheKey::ConicalFrustum {
                radius_top_milli: quantize_milli(radius_top),
                radius_bottom_milli: quantize_milli(radius_bottom),
                height_milli: quantize_milli(height),
            },
            PrimitiveParams::Torus {
                minor_radius,
                major_radius,
            } => PrimitiveMeshCacheKey::Torus {
                minor_radius_milli: quantize_milli(minor_radius),
                major_radius_milli: quantize_milli(major_radius),
            },
        };

        if let Some(existing) = self.map.get(&key) {
            return existing.clone();
        }

        let handle = match params {
            PrimitiveParams::Capsule {
                radius,
                half_length,
            } => meshes.add(Capsule3d::new(radius, half_length)),
            PrimitiveParams::ConicalFrustum {
                radius_top,
                radius_bottom,
                height,
            } => meshes.add(ConicalFrustum {
                radius_top,
                radius_bottom,
                height,
            }),
            PrimitiveParams::Torus {
                minor_radius,
                major_radius,
            } => meshes.add(Torus::new(minor_radius, major_radius)),
        };

        self.map.insert(key, handle.clone());
        handle
    }

    fn get_or_create_mirrored_winding(
        &mut self,
        meshes: &mut Assets<Mesh>,
        mesh: &Handle<Mesh>,
    ) -> Handle<Mesh> {
        let key = mesh.id();
        if let Some(existing) = self.mirrored_winding.get(&key) {
            return existing.clone();
        }

        let Some(base) = meshes.get(mesh).cloned() else {
            return mesh.clone();
        };
        let mut mirrored = base;

        match mirrored.try_indices_option() {
            Ok(Some(_)) => {}
            Ok(None) => {
                if !matches!(
                    mirrored.primitive_topology(),
                    bevy::render::render_resource::PrimitiveTopology::TriangleList
                ) {
                    return mesh.clone();
                }
                let vertex_count = mirrored.count_vertices();
                if !vertex_count.is_multiple_of(3) {
                    return mesh.clone();
                }
                let indices = (0..vertex_count as u32).collect::<Vec<_>>();
                mirrored.insert_indices(bevy::mesh::Indices::U32(indices));
            }
            Err(_) => return mesh.clone(),
        }

        if mirrored.invert_winding().is_err() {
            return mesh.clone();
        }

        let handle = meshes.add(mirrored);
        self.mirrored_winding.insert(key, handle.clone());
        handle
    }
}

#[derive(Component, Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct VisualPartId(pub(crate) u128);

#[derive(Component, Clone, Debug)]
pub(crate) struct PartAnimationPlayer {
    pub(crate) root_entity: Entity,
    pub(crate) parent_object_id: u128,
    pub(crate) child_object_id: Option<u128>,
    pub(crate) attachment: Option<AttachmentDef>,
    pub(crate) base_transform: Transform,
    pub(crate) animations: Vec<PartAnimationSlot>,
    pub(crate) apply_aim_yaw: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct VisualSpawnSettings {
    pub(crate) mark_parts: bool,
    pub(crate) render_layer: Option<usize>,
}

#[derive(Component)]
pub(crate) struct PendingSceneOverrides {
    tint: Option<Color>,
    render_layer: Option<usize>,
}

pub(crate) fn spawn_object_visuals(
    entity: &mut EntityCommands<'_>,
    library: &ObjectLibrary,
    asset_server: &AssetServer,
    assets: &SceneAssets,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    material_cache: &mut MaterialCache,
    mesh_cache: &mut PrimitiveMeshCache,
    object_id: u128,
    tint: Option<Color>,
) {
    spawn_object_visuals_with_settings(
        entity,
        library,
        asset_server,
        assets,
        meshes,
        materials,
        material_cache,
        mesh_cache,
        object_id,
        tint,
        VisualSpawnSettings::default(),
    );
}

pub(crate) fn spawn_object_visuals_with_settings(
    entity: &mut EntityCommands<'_>,
    library: &ObjectLibrary,
    asset_server: &AssetServer,
    assets: &SceneAssets,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    material_cache: &mut MaterialCache,
    mesh_cache: &mut PrimitiveMeshCache,
    object_id: u128,
    tint: Option<Color>,
    settings: VisualSpawnSettings,
) {
    let root_entity = entity.id();
    let aim_object_ids = aim_object_ids_for_root(library, object_id);
    let mut stack = Vec::new();
    spawn_object_visuals_inner(
        entity,
        library,
        asset_server,
        assets,
        meshes,
        materials,
        material_cache,
        mesh_cache,
        object_id,
        tint,
        settings,
        &mut stack,
        root_entity,
        0,
        None,
        &aim_object_ids,
        false,
        false,
    );
}

fn aim_object_ids_for_root(library: &ObjectLibrary, root_object_id: u128) -> HashSet<u128> {
    let mut out = HashSet::new();
    let Some(def) = library.get(root_object_id) else {
        return out;
    };

    if let Some(aim) = def.aim.as_ref() {
        out.extend(aim.components.iter().copied());
    }

    if out.is_empty() {
        if let Some(attack) = def.attack.as_ref() {
            if matches!(
                attack.kind,
                crate::object::registry::UnitAttackKind::RangedProjectile
            ) {
                if let Some(ranged) = attack.ranged.as_ref() {
                    out.insert(ranged.muzzle.object_id);
                }
            }
        }
    }

    out
}

fn spawn_object_visuals_inner(
    entity: &mut EntityCommands<'_>,
    library: &ObjectLibrary,
    asset_server: &AssetServer,
    assets: &SceneAssets,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    material_cache: &mut MaterialCache,
    mesh_cache: &mut PrimitiveMeshCache,
    object_id: u128,
    tint: Option<Color>,
    settings: VisualSpawnSettings,
    stack: &mut Vec<u128>,
    root_entity: Entity,
    depth: usize,
    active_part_id: Option<u128>,
    aim_object_ids: &HashSet<u128>,
    ancestor_apply_aim_yaw: bool,
    ancestor_mirrored: bool,
) {
    if depth > MAX_VISUAL_DEPTH {
        warn!("Object visuals: max depth exceeded at object_id {object_id:#x}");
        return;
    }

    if stack.contains(&object_id) {
        warn!(
            "Object visuals: detected composition cycle: {:?} -> {object_id:#x}",
            stack
        );
        return;
    }

    let Some(def) = library.get(object_id) else {
        warn!("Object visuals: missing object_id {object_id:#x}");
        return;
    };

    stack.push(object_id);

    if def.parts.is_empty() {
        stack.pop();
        return;
    }

    entity.with_children(|parent| {
        for part in def.parts.iter() {
            let part_part_id = active_part_id.or(part.part_id);
            let mut child_transform = part.transform;
            if let Some(attachment) = part.attachment.as_ref() {
                child_transform = resolve_attachment_transform(library, def, part, attachment)
                    .unwrap_or_else(|| part.transform);
            }

            let local_det =
                child_transform.scale.x * child_transform.scale.y * child_transform.scale.z;
            let local_mirrored = local_det.is_finite() && local_det < 0.0;
            let child_mirrored = ancestor_mirrored ^ local_mirrored;

            let mut child = parent.spawn((child_transform, Visibility::Inherited));
            let apply_aim_yaw = !ancestor_apply_aim_yaw
                && match &part.kind {
                    ObjectPartKind::ObjectRef { object_id } => aim_object_ids.contains(object_id),
                    _ => false,
                };
            if !part.animations.is_empty() || apply_aim_yaw {
                let child_object_id = match &part.kind {
                    ObjectPartKind::ObjectRef { object_id } => Some(*object_id),
                    _ => None,
                };
                child.insert(PartAnimationPlayer {
                    root_entity,
                    parent_object_id: def.object_id,
                    child_object_id,
                    attachment: part.attachment.clone(),
                    base_transform: part.transform,
                    animations: part.animations.clone(),
                    apply_aim_yaw,
                });
            }
            match &part.kind {
                ObjectPartKind::ObjectRef {
                    object_id: child_id,
                } => {
                    spawn_object_visuals_inner(
                        &mut child,
                        library,
                        asset_server,
                        assets,
                        meshes,
                        materials,
                        material_cache,
                        mesh_cache,
                        *child_id,
                        tint,
                        settings,
                        stack,
                        root_entity,
                        depth + 1,
                        part_part_id,
                        aim_object_ids,
                        ancestor_apply_aim_yaw || apply_aim_yaw,
                        child_mirrored,
                    );
                }
                ObjectPartKind::Primitive { primitive } => {
                    if let Some((mesh, material)) = resolve_primitive_visual(
                        primitive,
                        tint,
                        assets,
                        meshes,
                        materials,
                        material_cache,
                        mesh_cache,
                    ) {
                        let mesh = if child_mirrored {
                            mesh_cache.get_or_create_mirrored_winding(meshes, &mesh)
                        } else {
                            mesh
                        };
                        child.insert((Mesh3d(mesh), MeshMaterial3d(material)));
                        if let Some(layer) = settings.render_layer {
                            child.insert(bevy::camera::visibility::RenderLayers::layer(layer));
                        }
                        if settings.mark_parts {
                            if let Some(part_id) = part_part_id {
                                child.insert(VisualPartId(part_id));
                            }
                        }
                    }
                }
                ObjectPartKind::Model { scene } => {
                    let handle: Handle<Scene> = asset_server.load(scene.clone().into_owned());
                    child.insert(SceneRoot(handle));
                    if settings.mark_parts {
                        if let Some(part_id) = part_part_id {
                            child.insert(VisualPartId(part_id));
                        }
                    }
                    if tint.is_some() || settings.render_layer.is_some() {
                        child.insert(PendingSceneOverrides {
                            tint,
                            render_layer: settings.render_layer,
                        });
                    }
                    if let Some(layer) = settings.render_layer {
                        child.insert(bevy::camera::visibility::RenderLayers::layer(layer));
                    }
                }
            }
        }
    });

    stack.pop();
}

fn resolve_attachment_transform(
    library: &ObjectLibrary,
    parent_def: &crate::object::registry::ObjectDef,
    part: &crate::object::registry::ObjectPartDef,
    attachment: &AttachmentDef,
) -> Option<Transform> {
    let parent_anchor = anchor_transform(parent_def, attachment.parent_anchor.as_ref())?;
    let child_anchor = match &part.kind {
        ObjectPartKind::ObjectRef { object_id } => library
            .get(*object_id)
            .and_then(|def| anchor_transform(def, attachment.child_anchor.as_ref()))
            .unwrap_or(Transform::IDENTITY),
        _ => Transform::IDENTITY,
    };

    let parent_mat = parent_anchor.to_matrix();
    let offset_mat = part.transform.to_matrix();
    let child_mat = child_anchor.to_matrix();
    let child_inv = child_mat.inverse();

    let composed = parent_mat * offset_mat * child_inv;
    let (scale, rotation, translation) = composed.to_scale_rotation_translation();
    if !translation.is_finite() || !rotation.is_finite() || !scale.is_finite() {
        return None;
    }
    Some(Transform {
        translation,
        rotation,
        scale,
    })
}

fn resolve_attachment_transform_with_offset(
    library: &ObjectLibrary,
    parent_def: &crate::object::registry::ObjectDef,
    child_object_id: Option<u128>,
    attachment: &AttachmentDef,
    offset: &Transform,
) -> Option<Transform> {
    let parent_anchor = anchor_transform(parent_def, attachment.parent_anchor.as_ref())?;
    let child_anchor = child_object_id
        .and_then(|object_id| {
            library
                .get(object_id)
                .and_then(|def| anchor_transform(def, attachment.child_anchor.as_ref()))
        })
        .unwrap_or(Transform::IDENTITY);

    let composed =
        parent_anchor.to_matrix() * offset.to_matrix() * child_anchor.to_matrix().inverse();
    let (scale, rotation, translation) = composed.to_scale_rotation_translation();
    if !translation.is_finite() || !rotation.is_finite() || !scale.is_finite() {
        return None;
    }
    Some(Transform {
        translation,
        rotation,
        scale,
    })
}

fn anchor_transform(def: &crate::object::registry::ObjectDef, name: &str) -> Option<Transform> {
    if name == "origin" {
        return Some(Transform::IDENTITY);
    }
    def.anchors
        .iter()
        .find(|a| a.name.as_ref() == name)
        .map(|a| a.transform)
}

fn resolve_primitive_visual(
    visual: &PrimitiveVisualDef,
    tint: Option<Color>,
    assets: &SceneAssets,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    material_cache: &mut MaterialCache,
    mesh_cache: &mut PrimitiveMeshCache,
) -> Option<(Handle<Mesh>, Handle<StandardMaterial>)> {
    let tint = tint.unwrap_or(Color::WHITE);

    match visual {
        PrimitiveVisualDef::Mesh { mesh, material } => {
            let mesh = resolve_mesh(*mesh, assets)?;
            let material = resolve_material(*material, assets)?;
            let material = material_cache.get_or_create_tinted(materials, &material, tint);
            Some((mesh, material))
        }
        PrimitiveVisualDef::Primitive {
            mesh,
            params,
            color,
            unlit,
        } => {
            let mesh = match params {
                Some(params)
                    if matches!(
                        (*mesh, params),
                        (MeshKey::UnitCapsule, PrimitiveParams::Capsule { .. })
                            | (
                                MeshKey::UnitConicalFrustum,
                                PrimitiveParams::ConicalFrustum { .. }
                            )
                            | (MeshKey::UnitTorus, PrimitiveParams::Torus { .. })
                    ) =>
                {
                    mesh_cache.get_or_create(meshes, *params)
                }
                Some(_) => resolve_mesh(*mesh, assets)?,
                None => resolve_mesh(*mesh, assets)?,
            };

            let color = multiply_color(*color, tint);
            let material = material_cache.get_or_create_color(materials, color, *unlit);
            Some((mesh, material))
        }
    }
}

fn resolve_mesh(key: MeshKey, assets: &SceneAssets) -> Option<Handle<Mesh>> {
    Some(match key {
        MeshKey::UnitCube => assets.unit_cube_mesh.clone(),
        MeshKey::UnitCylinder => assets.unit_cylinder_mesh.clone(),
        MeshKey::UnitCone => assets.unit_cone_mesh.clone(),
        MeshKey::UnitSphere => assets.unit_sphere_mesh.clone(),
        MeshKey::UnitPlane => assets.unit_plane_mesh.clone(),
        MeshKey::UnitCapsule => assets.unit_capsule_mesh.clone(),
        MeshKey::UnitConicalFrustum => assets.unit_conical_frustum_mesh.clone(),
        MeshKey::UnitTorus => assets.unit_torus_mesh.clone(),
        MeshKey::UnitTriangle => assets.unit_triangle_mesh.clone(),
        MeshKey::UnitTetrahedron => assets.unit_tetrahedron_mesh.clone(),
        MeshKey::TreeTrunk => assets.tree_trunk_mesh.clone(),
        MeshKey::TreeCone => assets.tree_cone_mesh.clone(),
    })
}

fn resolve_material(key: MaterialKey, assets: &SceneAssets) -> Option<Handle<StandardMaterial>> {
    let material = match key {
        MaterialKey::BuildBlock { index } => assets
            .build_block_materials
            .get(index)
            .cloned()
            .or_else(|| assets.build_block_materials.first().cloned())?,
        MaterialKey::FenceStake => assets.fence_stake_material.clone(),
        MaterialKey::FenceStick => assets.fence_stick_material.clone(),
        MaterialKey::TreeTrunk { variant } => assets
            .tree_trunk_materials
            .get(variant)
            .cloned()
            .or_else(|| assets.tree_trunk_materials.first().cloned())?,
        MaterialKey::TreeMain { variant } => assets
            .tree_main_materials
            .get(variant)
            .cloned()
            .or_else(|| assets.tree_main_materials.first().cloned())?,
        MaterialKey::TreeCrown { variant } => assets
            .tree_crown_materials
            .get(variant)
            .cloned()
            .or_else(|| assets.tree_crown_materials.first().cloned())?,
    };
    Some(material)
}

fn multiply_color(base: Color, tint: Color) -> Color {
    let a = base.to_srgba();
    let b = tint.to_srgba();
    Color::srgba(
        a.red * b.red,
        a.green * b.green,
        a.blue * b.blue,
        a.alpha * b.alpha,
    )
}

fn to_rgba8(color: Color) -> [u8; 4] {
    let srgba = color.to_srgba();
    [
        (srgba.red.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
        (srgba.green.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
        (srgba.blue.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
        (srgba.alpha.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
    ]
}

fn signed_move_distance_for_spin_axis(
    library: &ObjectLibrary,
    player: &PartAnimationPlayer,
    axis: Vec3,
    distance_m: f32,
) -> f32 {
    const EPS: f32 = 1e-6;
    // Only flip when the axis is clearly pointing towards -X in the parent's local space.
    // This fixes common vehicle cases where left wheels (axis ~ -X) otherwise spin backwards
    // relative to right wheels (axis ~ +X).
    const FLIP_DOT_THRESHOLD: f32 = -0.5;

    if !distance_m.is_finite() {
        return distance_m;
    }

    let axis = if axis.length_squared() > EPS {
        axis.normalize()
    } else {
        Vec3::Y
    };

    // We only apply this heuristic for attached parts. Unattached spinners (internal geometry)
    // can legitimately use arbitrary spin conventions, and flipping them can invert correct
    // animations.
    let Some(attachment) = player.attachment.as_ref() else {
        return distance_m;
    };

    let base_rot = if player.base_transform.rotation.is_finite() {
        player.base_transform.rotation.normalize()
    } else {
        Quat::IDENTITY
    };
    let mut axis_in_parent_root = base_rot * axis;

    if let Some(parent_def) = library.get(player.parent_object_id) {
        if let Some(parent_anchor) = anchor_transform(parent_def, attachment.parent_anchor.as_ref())
        {
            let parent_rot = if parent_anchor.rotation.is_finite() {
                parent_anchor.rotation.normalize()
            } else {
                Quat::IDENTITY
            };
            axis_in_parent_root = parent_rot * axis_in_parent_root;
        }
    }

    let axis_in_parent_root = if axis_in_parent_root.length_squared() > EPS {
        axis_in_parent_root.normalize()
    } else {
        axis_in_parent_root
    };

    let dot_x = axis_in_parent_root.dot(Vec3::X);
    if dot_x < FLIP_DOT_THRESHOLD {
        -distance_m
    } else {
        distance_m
    }
}

pub(crate) fn update_part_animations(
    time: Res<Time>,
    library: Res<ObjectLibrary>,
    prefabs: Query<&ObjectPrefabId>,
    locomotion: Query<&LocomotionClock>,
    attacks: Query<&AttackClock>,
    channels: Query<&AnimationChannelsActive>,
    forced: Query<&ForcedAnimationChannel>,
    aim_deltas: Query<&crate::types::AimYawDelta>,
    mut q: Query<(&PartAnimationPlayer, &mut Transform)>,
) {
    let wall_time = time.elapsed_secs();
    for (player, mut transform) in q.iter_mut() {
        let active = channels
            .get(player.root_entity)
            .ok()
            .copied()
            .unwrap_or_default();

        let attack_active = active.attacking_primary;
        let move_active = active.moving;
        let idle_active = !attack_active && !move_active;

        let mut chosen: Option<&PartAnimationSpec> = None;

        // If the root entity has a forced channel override, prefer it when this part has a
        // matching slot.
        let forced_channel = forced
            .get(player.root_entity)
            .ok()
            .map(|c| c.channel.trim())
            .filter(|c| !c.is_empty());
        if let Some(channel) = forced_channel {
            if let Some(slot) = player
                .animations
                .iter()
                .find(|slot| slot.channel.as_ref() == channel)
            {
                chosen = Some(&slot.spec);
            }
        }

        if chosen.is_none() {
            for channel in ["attack_primary", "move", "idle", "ambient"] {
                let channel_active = match channel {
                    "attack_primary" => attack_active,
                    "move" => move_active,
                    "idle" => idle_active,
                    // Ambient is always active (fallback animation like fans/spinners).
                    "ambient" => true,
                    _ => false,
                };
                if !channel_active {
                    continue;
                }
                if let Some(slot) = player
                    .animations
                    .iter()
                    .find(|slot| slot.channel.as_ref() == channel)
                {
                    chosen = Some(&slot.spec);
                    break;
                }
            }
        }

        let spec = chosen;

        let allow_aim_yaw = if player.apply_aim_yaw {
            // Melee units look better when they *only* apply attention yaw during the active
            // attack window; otherwise the weapon can appear "stuck" pointing at the target
            // between swings.
            let is_melee = prefabs
                .get(player.root_entity)
                .ok()
                .and_then(|prefab_id| library.get(prefab_id.0))
                .and_then(|def| def.attack.as_ref())
                .map(|attack| attack.kind == UnitAttackKind::Melee)
                .unwrap_or(false);
            !is_melee || attack_active
        } else {
            false
        };

        let aim_delta = if allow_aim_yaw {
            aim_deltas
                .get(player.root_entity)
                .ok()
                .map(|v| v.0)
                .unwrap_or(0.0)
        } else {
            0.0
        };
        let aim_quat_parent = if aim_delta.is_finite() {
            Quat::from_rotation_y(aim_delta)
        } else {
            Quat::IDENTITY
        };

        let mut base = player.base_transform;
        if allow_aim_yaw {
            // `AimYawDelta` is expressed in the root body's frame (+Y is up). For attachments,
            // the join frame at `parent_anchor` can be arbitrarily oriented (e.g. a neck anchor's
            // +Z points "out of the joint", not necessarily world-forward). Convert the yaw
            // rotation into the join frame so aim always yaws around the parent's vertical axis.
            let aim_quat = if let Some(attachment) = player.attachment.as_ref() {
                library
                    .get(player.parent_object_id)
                    .and_then(|parent_def| {
                        anchor_transform(parent_def, attachment.parent_anchor.as_ref())
                    })
                    .map(|anchor| {
                        let anchor_rot = if anchor.rotation.is_finite() {
                            anchor.rotation.normalize()
                        } else {
                            Quat::IDENTITY
                        };
                        let q = anchor_rot.inverse() * aim_quat_parent * anchor_rot;
                        if q.is_finite() {
                            q.normalize()
                        } else {
                            Quat::IDENTITY
                        }
                    })
                    .unwrap_or(aim_quat_parent)
            } else {
                aim_quat_parent
            };

            base.rotation = aim_quat * base.rotation;
        }

        let delta = if let Some(spec) = spec {
            let driver_time = match spec.driver {
                PartAnimationDriver::Always => wall_time,
                PartAnimationDriver::MovePhase => locomotion
                    .get(player.root_entity)
                    .map(|clock| clock.t)
                    .unwrap_or(0.0),
                PartAnimationDriver::MoveDistance => locomotion
                    .get(player.root_entity)
                    .map(|clock| match &spec.clip {
                        PartAnimationDef::Spin { axis, .. } => signed_move_distance_for_spin_axis(
                            &library,
                            player,
                            *axis,
                            clock.signed_distance_m,
                        ),
                        _ => clock.distance_m,
                    })
                    .unwrap_or(0.0),
                PartAnimationDriver::AttackTime => attacks
                    .get(player.root_entity)
                    .map(|clock| {
                        if clock.duration_secs > 0.0 {
                            (wall_time - clock.started_at_secs).max(0.0)
                        } else {
                            0.0
                        }
                    })
                    .unwrap_or(0.0),
            };

            let mut t = driver_time * spec.speed_scale.max(0.0);
            if spec.time_offset_units.is_finite() {
                t += spec.time_offset_units;
            }
            sample_part_animation(&spec.clip, t)
        } else {
            Transform::IDENTITY
        };
        let animated_base = mul_transform(&base, &delta);

        if let Some(attachment) = player.attachment.as_ref() {
            let Some(parent_def) = library.get(player.parent_object_id) else {
                continue;
            };
            if let Some(local) = resolve_attachment_transform_with_offset(
                &library,
                parent_def,
                player.child_object_id,
                attachment,
                &animated_base,
            ) {
                *transform = local;
            } else {
                *transform = animated_base;
            }
        } else {
            *transform = animated_base;
        }
    }
}

fn sample_part_animation(animation: &PartAnimationDef, time_secs: f32) -> Transform {
    match animation {
        PartAnimationDef::Loop {
            duration_secs,
            keyframes,
        } => {
            let duration = (*duration_secs).max(1e-6);
            let mut t = if time_secs.is_finite() {
                time_secs
            } else {
                0.0
            };
            t = t.rem_euclid(duration);

            if keyframes.is_empty() {
                return Transform::IDENTITY;
            }
            if keyframes.len() == 1 {
                return keyframes[0].delta;
            }

            let mut prev = &keyframes[0];
            for next in &keyframes[1..] {
                if t < next.time_secs {
                    let dt = (next.time_secs - prev.time_secs).max(1e-6);
                    let alpha = ((t - prev.time_secs) / dt).clamp(0.0, 1.0);
                    return lerp_transform(&prev.delta, &next.delta, alpha);
                }
                prev = next;
            }

            // Wrap around (last -> first).
            let first = &keyframes[0];
            let last = prev;
            let t0 = last.time_secs;
            let t1 = duration + first.time_secs;
            let dt = (t1 - t0).max(1e-6);
            let alpha = ((t - t0) / dt).clamp(0.0, 1.0);
            lerp_transform(&last.delta, &first.delta, alpha)
        }
        PartAnimationDef::Spin {
            axis,
            radians_per_unit,
        } => {
            let axis = if axis.length_squared() > 1e-6 {
                axis.normalize()
            } else {
                Vec3::Y
            };
            let angle = if time_secs.is_finite() && radians_per_unit.is_finite() {
                time_secs * *radians_per_unit
            } else {
                0.0
            };
            Transform {
                translation: Vec3::ZERO,
                rotation: Quat::from_axis_angle(axis, angle),
                scale: Vec3::ONE,
            }
        }
    }
}

fn lerp_transform(a: &Transform, b: &Transform, alpha: f32) -> Transform {
    let translation = a.translation.lerp(b.translation, alpha);
    let rotation = a.rotation.slerp(b.rotation, alpha).normalize();
    let scale = a.scale.lerp(b.scale, alpha);
    Transform {
        translation,
        rotation,
        scale,
    }
}

fn mul_transform(a: &Transform, b: &Transform) -> Transform {
    let composed = a.to_matrix() * b.to_matrix();
    let (scale, rotation, translation) = composed.to_scale_rotation_translation();
    if !translation.is_finite() || !rotation.is_finite() || !scale.is_finite() {
        return *b;
    }
    Transform {
        translation,
        rotation,
        scale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrored_winding_cache_inverts_triangle_indices() {
        use bevy::asset::RenderAssetUsages;
        use bevy::mesh::Indices;
        use bevy::render::render_resource::PrimitiveTopology;

        let mut meshes: Assets<Mesh> = Assets::default();
        let mut cache = PrimitiveMeshCache::default();

        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        let handle = meshes.add(mesh);

        let mirrored_handle = cache.get_or_create_mirrored_winding(&mut meshes, &handle);
        assert_ne!(
            handle.id(),
            mirrored_handle.id(),
            "expected mirrored winding mesh to be a distinct asset"
        );

        let mirrored = meshes
            .get(&mirrored_handle)
            .expect("mirrored mesh exists in assets");
        let Some(Indices::U32(indices)) = mirrored.indices() else {
            panic!("expected mirrored mesh to have U32 indices");
        };
        assert_eq!(indices.as_slice(), &[0, 2, 1]);

        let mirrored_handle_2 = cache.get_or_create_mirrored_winding(&mut meshes, &handle);
        assert_eq!(
            mirrored_handle.id(),
            mirrored_handle_2.id(),
            "expected mirrored winding meshes to be cached per base mesh"
        );
    }

    #[test]
    fn mul_transform_keeps_base_translation_when_applying_rotation_delta() {
        let base = Transform::from_translation(Vec3::new(1.0, 0.0, 0.0));
        let delta = Transform {
            rotation: Quat::from_rotation_y(core::f32::consts::FRAC_PI_2),
            ..default()
        };

        // Animation deltas should be applied in the part's local space:
        // `animated = base * delta`. In particular, a pure rotation delta should not rotate the
        // base translation (which would make limbs orbit around the parent origin).
        let animated = mul_transform(&base, &delta);

        assert!(
            (animated.translation - base.translation).length() < 1e-4,
            "base translation rotated unexpectedly: base={:?} animated={:?}",
            base.translation,
            animated.translation
        );
    }

    #[test]
    fn move_distance_spin_flips_when_axis_points_left() {
        use crate::object::registry::{ColliderProfile, ObjectDef, ObjectInteraction};

        fn anchor(name: &str, forward: Vec3) -> crate::object::registry::AnchorDef {
            let rot = if forward.dot(Vec3::X) >= 0.0 {
                Quat::from_rotation_y(core::f32::consts::FRAC_PI_2)
            } else {
                Quat::from_rotation_y(-core::f32::consts::FRAC_PI_2)
            };
            crate::object::registry::AnchorDef {
                name: name.to_string().into(),
                transform: Transform::from_rotation(rot),
            }
        }

        let parent_id = 0xdead_beef_u128;
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
            anchors: vec![
                anchor("mount_left", Vec3::NEG_X),
                anchor("mount_right", Vec3::X),
            ],
            parts: Vec::new(),
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        });

        let player_left = PartAnimationPlayer {
            root_entity: Entity::from_bits(1),
            parent_object_id: parent_id,
            child_object_id: None,
            attachment: Some(AttachmentDef {
                parent_anchor: "mount_left".into(),
                child_anchor: "origin".into(),
            }),
            base_transform: Transform::IDENTITY,
            animations: Vec::new(),
            apply_aim_yaw: false,
        };

        let player_right = PartAnimationPlayer {
            attachment: Some(AttachmentDef {
                parent_anchor: "mount_right".into(),
                child_anchor: "origin".into(),
            }),
            ..player_left.clone()
        };

        // Stored spin axis is in the attachment-local frame; +Z maps to the mount's `forward`.
        let axis_local = Vec3::Z;
        let left = signed_move_distance_for_spin_axis(&library, &player_left, axis_local, 10.0);
        let right = signed_move_distance_for_spin_axis(&library, &player_right, axis_local, 10.0);

        assert!(
            left < 0.0,
            "expected left distance to flip negative, got {left}"
        );
        assert!(
            right > 0.0,
            "expected right distance to stay positive, got {right}"
        );

        let left = signed_move_distance_for_spin_axis(&library, &player_left, axis_local, -10.0);
        let right = signed_move_distance_for_spin_axis(&library, &player_right, axis_local, -10.0);

        assert!(
            left > 0.0,
            "expected left distance to flip positive, got {left}"
        );
        assert!(
            right < 0.0,
            "expected right distance to stay negative, got {right}"
        );
    }

    #[test]
    fn aim_yaw_applies_in_parent_frame_even_when_anchor_frame_is_rotated() {
        use crate::object::registry::{AnchorDef, ColliderProfile, ObjectDef, ObjectInteraction};
        use crate::types::AimYawDelta;

        // A rotated anchor frame where +Z points up (common for "neck"/"shoulder" style joints).
        // If we naively apply yaw in the *anchor* frame, we'd rotate around a horizontal axis.
        let anchor_rot =
            Quat::from_mat3(&Mat3::from_cols(Vec3::NEG_X, Vec3::Z, Vec3::Y)).normalize();

        let parent_id = 0xfeed_face_u128;
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
                name: "neck".into(),
                transform: Transform::from_rotation(anchor_rot),
            }],
            parts: Vec::new(),
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        });

        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(library);
        app.add_systems(Update, update_part_animations);

        let aim_delta = core::f32::consts::FRAC_PI_2;
        let expected = Quat::from_rotation_y(aim_delta);

        let root = app.world_mut().spawn(AimYawDelta(aim_delta)).id();
        let part = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                PartAnimationPlayer {
                    root_entity: root,
                    parent_object_id: parent_id,
                    child_object_id: None,
                    attachment: Some(AttachmentDef {
                        parent_anchor: "neck".into(),
                        child_anchor: "origin".into(),
                    }),
                    base_transform: Transform::IDENTITY,
                    animations: Vec::new(),
                    apply_aim_yaw: true,
                },
            ))
            .id();

        app.update();

        let transform = app
            .world()
            .get::<Transform>(part)
            .copied()
            .expect("part entity has Transform");

        // Rest pose without yaw would be `anchor_rot`. The yaw delta should apply in the parent
        // frame, so the relative delta should be exactly the expected yaw quaternion.
        let delta = (transform.rotation * anchor_rot.inverse()).normalize();
        assert!(
            delta.angle_between(expected) < 1e-3,
            "expected aim yaw delta to be applied in parent frame: anchor_rot={:?} delta={:?} expected={:?}",
            anchor_rot,
            delta,
            expected,
        );
    }

    #[test]
    fn melee_aim_yaw_only_applies_during_attack_window() {
        use crate::object::registry::{
            ColliderProfile, MeleeAttackProfile, ObjectDef, ObjectInteraction, UnitAttackProfile,
        };
        use crate::types::{AimYawDelta, ObjectPrefabId};

        let root_object_id = 0xabc0_def1_u128;
        let mut library = ObjectLibrary::default();
        library.upsert(ObjectDef {
            object_id: root_object_id,
            label: "root".into(),
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
            attack: Some(UnitAttackProfile {
                kind: crate::object::registry::UnitAttackKind::Melee,
                cooldown_secs: 0.0,
                damage: 0,
                anim_window_secs: 0.25,
                melee: Some(MeleeAttackProfile {
                    range: 1.0,
                    radius: 0.5,
                    arc_degrees: 90.0,
                }),
                ranged: None,
            }),
        });

        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(library);
        app.add_systems(Update, update_part_animations);

        let aim_delta = core::f32::consts::FRAC_PI_2;
        let root = app
            .world_mut()
            .spawn((
                ObjectPrefabId(root_object_id),
                AimYawDelta(aim_delta),
                AnimationChannelsActive {
                    moving: false,
                    attacking_primary: false,
                },
            ))
            .id();

        let part = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                PartAnimationPlayer {
                    root_entity: root,
                    parent_object_id: root_object_id,
                    child_object_id: None,
                    attachment: None,
                    base_transform: Transform::IDENTITY,
                    animations: Vec::new(),
                    apply_aim_yaw: true,
                },
            ))
            .id();

        app.update();
        let transform = app
            .world()
            .get::<Transform>(part)
            .copied()
            .expect("part entity has Transform");
        assert!(
            transform.rotation.angle_between(Quat::IDENTITY) < 1e-3,
            "expected melee aim yaw to be suppressed when not attacking; rot={:?}",
            transform.rotation
        );

        app.world_mut()
            .entity_mut(root)
            .insert(AnimationChannelsActive {
                moving: false,
                attacking_primary: true,
            });
        app.update();
        let transform = app
            .world()
            .get::<Transform>(part)
            .copied()
            .expect("part entity has Transform");
        let expected = Quat::from_rotation_y(aim_delta);
        assert!(
            transform.rotation.angle_between(expected) < 1e-3,
            "expected melee aim yaw to apply during attack window; rot={:?} expected={:?}",
            transform.rotation,
            expected
        );
    }

    #[test]
    fn no_fallback_animation_when_channel_missing() {
        use crate::object::registry::{
            PartAnimationDef, PartAnimationDriver, PartAnimationKeyframeDef, PartAnimationSlot,
            PartAnimationSpec,
        };
        use std::time::Duration;

        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(ObjectLibrary::default());
        app.add_systems(Update, update_part_animations);

        let root = app
            .world_mut()
            .spawn(AnimationChannelsActive {
                moving: true,
                attacking_primary: false,
            })
            .id();

        let spec = PartAnimationSpec {
            driver: PartAnimationDriver::Always,
            speed_scale: 1.0,
            time_offset_units: 0.0,
            clip: PartAnimationDef::Loop {
                duration_secs: 2.0,
                keyframes: vec![
                    PartAnimationKeyframeDef {
                        time_secs: 0.0,
                        delta: Transform::IDENTITY,
                    },
                    PartAnimationKeyframeDef {
                        time_secs: 1.0,
                        delta: Transform::from_translation(Vec3::new(0.0, 1.0, 0.0)),
                    },
                ],
            },
        };

        let part_entity = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                PartAnimationPlayer {
                    root_entity: root,
                    parent_object_id: 0,
                    child_object_id: None,
                    attachment: None,
                    base_transform: Transform::IDENTITY,
                    animations: vec![PartAnimationSlot {
                        channel: "idle".into(),
                        spec,
                    }],
                    apply_aim_yaw: false,
                },
            ))
            .id();

        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(1.0));
        app.update();

        let transform = app
            .world()
            .get::<Transform>(part_entity)
            .copied()
            .expect("part entity has Transform");
        assert!(
            transform.translation.length_squared() < 1e-6,
            "expected no fallback animation while moving, got translation={:?}",
            transform.translation
        );
    }

    #[test]
    fn move_animation_applies_time_offset_units() {
        use crate::object::registry::{
            PartAnimationDef, PartAnimationDriver, PartAnimationKeyframeDef, PartAnimationSlot,
            PartAnimationSpec,
        };

        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(ObjectLibrary::default());
        app.add_systems(Update, update_part_animations);

        let root = app
            .world_mut()
            .spawn(AnimationChannelsActive {
                moving: true,
                attacking_primary: false,
            })
            .id();

        let spec = PartAnimationSpec {
            driver: PartAnimationDriver::Always,
            speed_scale: 1.0,
            time_offset_units: 1.0,
            clip: PartAnimationDef::Loop {
                duration_secs: 2.0,
                keyframes: vec![
                    PartAnimationKeyframeDef {
                        time_secs: 0.0,
                        delta: Transform::IDENTITY,
                    },
                    PartAnimationKeyframeDef {
                        time_secs: 1.0,
                        delta: Transform::from_translation(Vec3::new(0.0, 1.0, 0.0)),
                    },
                ],
            },
        };

        let part_entity = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                PartAnimationPlayer {
                    root_entity: root,
                    parent_object_id: 0,
                    child_object_id: None,
                    attachment: None,
                    base_transform: Transform::IDENTITY,
                    animations: vec![PartAnimationSlot {
                        channel: "move".into(),
                        spec,
                    }],
                    apply_aim_yaw: false,
                },
            ))
            .id();

        // With wall_time=0 and time_offset_units=1, we should sample at t=1.0.
        app.update();

        let transform = app
            .world()
            .get::<Transform>(part_entity)
            .copied()
            .expect("part entity has Transform");
        assert!(
            (transform.translation.y - 1.0).abs() < 1e-4,
            "expected time_offset_units to phase-shift sampling, got translation={:?}",
            transform.translation
        );
    }
}

pub(crate) fn apply_pending_scene_overrides(
    mut commands: Commands,
    children_q: Query<&Children>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut material_cache: ResMut<MaterialCache>,
    mut scene_materials: Query<&mut MeshMaterial3d<StandardMaterial>>,
    pending_q: Query<(Entity, &PendingSceneOverrides)>,
) {
    for (root, overrides) in pending_q.iter() {
        let mut stack = vec![root];
        let mut any_mesh_found = false;

        while let Some(entity) = stack.pop() {
            if let Ok(children) = children_q.get(entity) {
                for child in children.iter() {
                    stack.push(child);
                }
            }

            if let Ok(mut material) = scene_materials.get_mut(entity) {
                any_mesh_found = true;
                if let Some(tint) = overrides.tint {
                    let tinted =
                        material_cache.get_or_create_tinted(&mut materials, &material.0, tint);
                    material.0 = tinted;
                }
            }

            if let Some(layer) = overrides.render_layer {
                commands
                    .entity(entity)
                    .insert(bevy::camera::visibility::RenderLayers::layer(layer));
            }
        }

        if any_mesh_found {
            commands.entity(root).remove::<PendingSceneOverrides>();
        }
    }
}
