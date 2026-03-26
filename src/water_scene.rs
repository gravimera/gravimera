use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::light::AtmosphereEnvironmentMapLight;
use bevy::pbr::{Atmosphere, AtmosphereSettings, ScatteringMedium};
use bevy::prelude::*;
use bevy_water::{WaterPlugin, WaterQuality, WaterSettings};

use crate::types::MainCamera;

pub(crate) struct WaterScenePlugin;

impl Plugin for WaterScenePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(WaterSettings {
            height: -0.2,
            amplitude: 0.25,
            spawn_tiles: Some(UVec2::new(2, 2)),
            water_quality: WaterQuality::Ultra,
            ..default()
        });

        app.add_plugins(WaterPlugin);
        app.add_systems(
            Startup,
            ensure_main_camera_depth_prepass.after(crate::setup::setup_rendered),
        );
        app.add_systems(
            Startup,
            ensure_main_camera_atmosphere.after(crate::setup::setup_rendered),
        );
    }
}

fn ensure_main_camera_depth_prepass(
    mut commands: Commands,
    cameras: Query<Entity, (With<Camera3d>, With<MainCamera>, Without<DepthPrepass>)>,
) {
    for entity in cameras.iter() {
        commands.entity(entity).insert(DepthPrepass);
    }
}

fn ensure_main_camera_atmosphere(
    mut commands: Commands,
    mut scattering_mediums: ResMut<Assets<ScatteringMedium>>,
    cameras: Query<Entity, (With<Camera3d>, With<MainCamera>, Without<Atmosphere>)>,
) {
    if cameras.is_empty() {
        return;
    }

    let medium = scattering_mediums.add(ScatteringMedium::default());
    for entity in cameras.iter() {
        commands.entity(entity).insert((
            Atmosphere::earthlike(medium.clone()),
            AtmosphereSettings::default(),
            AtmosphereEnvironmentMapLight::default(),
        ));
    }
}
