use bevy::prelude::*;

use crate::assets::SceneAssets;
use crate::object::registry::ObjectLibrary;
use crate::types::{BuildObject, Commandable, ObjectPrefabId, ObjectTint, Player, SceneLayerOwner};

#[derive(Component)]
pub(crate) struct SceneInstanceVisualsSpawned;
#[derive(Component)]
pub(crate) struct SceneInstanceVisualsPending;

pub(crate) fn ensure_scene_instance_visuals_spawned(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    assets: Res<SceneAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut material_cache: ResMut<crate::object::visuals::MaterialCache>,
    mut mesh_cache: ResMut<crate::object::visuals::PrimitiveMeshCache>,
    library: Res<ObjectLibrary>,
    objects: Query<
        (Entity, &ObjectPrefabId, Option<&ObjectTint>),
        (
            Without<SceneInstanceVisualsSpawned>,
            Without<Player>,
            Or<(With<BuildObject>, With<Commandable>)>,
            Or<(With<SceneLayerOwner>, With<SceneInstanceVisualsPending>)>,
        ),
    >,
) {
    for (entity, prefab_id, tint) in &objects {
        let tint = tint.map(|t| t.0);
        let mut ec = commands.entity(entity);
        ec.insert(Visibility::Inherited);
        crate::object::visuals::spawn_object_visuals(
            &mut ec,
            &library,
            &asset_server,
            &assets,
            &mut meshes,
            &mut materials,
            &mut material_cache,
            &mut mesh_cache,
            prefab_id.0,
            tint,
        );
        ec.insert(SceneInstanceVisualsSpawned);
        ec.remove::<SceneInstanceVisualsPending>();
    }
}
