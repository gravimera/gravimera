use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::light::AtmosphereEnvironmentMapLight;
use bevy::pbr::{Atmosphere, AtmosphereSettings, DistanceFog, FogFalloff, ScatteringMedium};
use bevy::prelude::*;
use bevy_water::{WaterPlugin, WaterQuality, WaterSettings};

use crate::types::MainCamera;

pub(crate) struct WaterScenePlugin;

impl Plugin for WaterScenePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(WaterSettings {
            height: -0.2,
            amplitude: 0.25,
            clarity: 0.22,
            deep_color: Color::srgba(0.08, 0.20, 0.40, 1.0),
            shallow_color: Color::srgba(0.20, 0.55, 0.75, 1.0),
            spawn_tiles: Some(UVec2::new(6, 6)),
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
        app.add_systems(
            Startup,
            ensure_main_camera_ocean_horizon.after(crate::setup::setup_rendered),
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
        let settings = AtmosphereSettings {
            transmittance_lut_size: UVec2::new(128, 64),
            multiscattering_lut_size: UVec2::new(16, 16),
            sky_view_lut_size: UVec2::new(200, 100),
            aerial_view_lut_size: UVec3::new(16, 16, 16),
            transmittance_lut_samples: 20,
            multiscattering_lut_dirs: 16,
            multiscattering_lut_samples: 10,
            sky_view_lut_samples: 8,
            aerial_view_lut_samples: 6,
            sky_max_samples: 8,
            ..default()
        };

        commands.entity(entity).insert((
            Atmosphere::earthlike(medium.clone()),
            settings,
            AtmosphereEnvironmentMapLight {
                size: UVec2::new(128, 128),
                ..default()
            },
        ));
    }
}

fn ensure_main_camera_ocean_horizon(
    mut commands: Commands,
    cameras: Query<Entity, (With<Camera3d>, With<MainCamera>, Without<DistanceFog>)>,
) {
    for entity in cameras.iter() {
        commands.entity(entity).insert(DistanceFog {
            // Keep the ocean/sky horizon soft, while leaving the playable area crisp.
            falloff: FogFalloff::from_visibility_colors(
                700.0,
                Color::srgb(0.35, 0.50, 0.66),
                Color::srgb(0.80, 0.84, 1.00),
            ),
            color: Color::srgb(0.10, 0.20, 0.40),
            ..default()
        });
    }
}
