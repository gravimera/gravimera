use bevy::camera::visibility::RenderLayers;
use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot, ScreenshotCaptured};
use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::orbit_capture;
use crate::realm_floor_packages;

const GENFLOOR_THUMBNAIL_LAYER: usize = 30;
const GENFLOOR_THUMBNAIL_WIDTH_PX: u32 = 256;
const GENFLOOR_THUMBNAIL_HEIGHT_PX: u32 = 256;
const GENFLOOR_THUMBNAIL_TIMEOUT_SECS: u64 = 5;

#[derive(Clone, Debug)]
struct GenfloorThumbnailRequest {
    realm_id: String,
    floor_id: u128,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct GenfloorThumbnailRequestKey {
    realm_id: String,
    floor_id: u128,
}

impl GenfloorThumbnailRequestKey {
    fn from_request(req: &GenfloorThumbnailRequest) -> Self {
        Self {
            realm_id: req.realm_id.clone(),
            floor_id: req.floor_id,
        }
    }
}

#[derive(Debug)]
struct GenfloorThumbnailCaptureProgress {
    expected: u32,
    completed: u32,
}

#[derive(Debug)]
struct GenfloorThumbnailCapture {
    realm_id: String,
    floor_id: u128,
    thumbnail_path: std::path::PathBuf,
    root: Entity,
    target: Handle<Image>,
    progress: Arc<Mutex<GenfloorThumbnailCaptureProgress>>,
    started_at: Instant,
    warned_timeout: bool,
}

#[derive(Resource, Default)]
pub(crate) struct GenfloorThumbnailCaptureRuntime {
    active: Option<GenfloorThumbnailCapture>,
    queue: VecDeque<GenfloorThumbnailRequest>,
    queued: HashSet<GenfloorThumbnailRequestKey>,
}

pub(crate) fn genfloor_queue_thumbnail_capture(
    runtime: &mut GenfloorThumbnailCaptureRuntime,
    realm_id: String,
    floor_id: u128,
) -> bool {
    let req = GenfloorThumbnailRequest { realm_id, floor_id };
    let key = GenfloorThumbnailRequestKey::from_request(&req);
    if runtime.queued.contains(&key) {
        return false;
    }
    runtime.queue.push_back(req);
    runtime.queued.insert(key);
    true
}

pub(crate) fn genfloor_thumbnail_capture_poll(
    mut commands: Commands,
    mut runtime: ResMut<GenfloorThumbnailCaptureRuntime>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut floor_library: ResMut<crate::floor_library_ui::FloorLibraryUiState>,
) {
    let mut finished_capture: Option<GenfloorThumbnailCapture> = None;

    if let Some(capture) = runtime.active.as_mut() {
        let done = match capture.progress.lock() {
            Ok(guard) => guard.completed >= guard.expected.max(1),
            Err(_) => true,
        };

        if !done
            && capture.started_at.elapsed() > Duration::from_secs(GENFLOOR_THUMBNAIL_TIMEOUT_SECS)
            && !capture.warned_timeout
        {
            capture.warned_timeout = true;
            warn!(
                "GenFloor: thumbnail capture is taking longer than {}s.",
                GENFLOOR_THUMBNAIL_TIMEOUT_SECS
            );
        }

        if done {
            finished_capture = runtime.active.take();
        } else {
            return;
        }
    }

    if let Some(capture) = finished_capture {
        let key = GenfloorThumbnailRequestKey {
            realm_id: capture.realm_id.clone(),
            floor_id: capture.floor_id,
        };
        runtime.queued.remove(&key);
        if std::fs::metadata(&capture.thumbnail_path).is_err() {
            debug!(
                "GenFloor: thumbnail capture finished but output is missing (terrain={}): {}",
                uuid::Uuid::from_u128(capture.floor_id),
                capture.thumbnail_path.display()
            );
        }
        floor_library.mark_models_dirty();
        cleanup_genfloor_thumbnail_capture(&mut commands, &mut images, capture);
    }

    if runtime.active.is_some() {
        return;
    }

    loop {
        let Some(req) = runtime.queue.pop_front() else {
            return;
        };
        let key = GenfloorThumbnailRequestKey::from_request(&req);
        match start_genfloor_thumbnail_capture(
            &mut commands,
            &mut images,
            &mut meshes,
            &mut materials,
            &req.realm_id,
            req.floor_id,
        ) {
            Ok(capture) => {
                runtime.active = Some(capture);
                break;
            }
            Err(err) => {
                runtime.queued.remove(&key);
                debug!("{err}");
            }
        }
    }
}

fn start_genfloor_thumbnail_capture(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    realm_id: &str,
    floor_id: u128,
) -> Result<GenfloorThumbnailCapture, String> {
    let def = realm_floor_packages::load_realm_floor_def(realm_id, floor_id)?;
    let thumbnail_path =
        realm_floor_packages::realm_floor_package_thumbnail_path(realm_id, floor_id);

    if let Some(parent) = thumbnail_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create thumbnail dir {}: {err}", parent.display()))?;
    }

    let target = orbit_capture::create_render_target(
        images,
        GENFLOOR_THUMBNAIL_WIDTH_PX,
        GENFLOOR_THUMBNAIL_HEIGHT_PX,
    );
    let aspect =
        GENFLOOR_THUMBNAIL_WIDTH_PX.max(1) as f32 / GENFLOOR_THUMBNAIL_HEIGHT_PX.max(1) as f32;
    let mut projection = bevy::camera::PerspectiveProjection::default();
    projection.aspect_ratio = aspect;
    let fov_y = projection.fov;
    let near = projection.near;

    let size_x = def.mesh.size_m[0].max(0.5);
    let size_z = def.mesh.size_m[1].max(0.5);
    let thickness = def.mesh.thickness_m.max(0.05);
    let half_extents = Vec3::new(size_x, thickness, size_z) * 0.5;
    let focus = Vec3::ZERO;

    let yaw = std::f32::consts::FRAC_PI_6;
    let pitch = -0.45;
    let base_distance =
        orbit_capture::required_distance_for_view(half_extents, yaw, pitch, fov_y, aspect, near);
    let distance = (base_distance * 1.1).clamp(near + 0.2, 500.0);
    let camera_transform = orbit_capture::orbit_transform(yaw, pitch, distance, focus);

    let render_layer = RenderLayers::layer(GENFLOOR_THUMBNAIL_LAYER);

    let root = commands
        .spawn((
            Transform::IDENTITY,
            Visibility::Inherited,
            render_layer.clone(),
        ))
        .id();

    let mesh_handle = meshes.add(crate::genfloor::build_floor_mesh_only(&def));
    let material = crate::genfloor::build_floor_material(&def, materials);
    let floor_id_entity = commands
        .spawn((
            Mesh3d(mesh_handle),
            MeshMaterial3d(material),
            Transform::IDENTITY,
            Visibility::Inherited,
            render_layer.clone(),
        ))
        .id();
    commands.entity(root).add_child(floor_id_entity);

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
            ))
            .id();
        commands.entity(root).add_child(light_id);
    }

    let camera_id = commands
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
        ))
        .id();
    commands.entity(root).add_child(camera_id);

    let progress = Arc::new(Mutex::new(GenfloorThumbnailCaptureProgress {
        expected: 1,
        completed: 0,
    }));
    let progress_for_capture = progress.clone();
    let path_for_capture = thumbnail_path.clone();
    let _screenshot = commands
        .spawn(Screenshot::image(target.clone()))
        .observe(move |event: On<ScreenshotCaptured>| {
            let mut saver = save_to_disk(path_for_capture.clone());
            saver(event);
            if let Ok(mut guard) = progress_for_capture.lock() {
                guard.completed = guard.completed.saturating_add(1);
            }
        })
        .id();

    Ok(GenfloorThumbnailCapture {
        realm_id: realm_id.to_string(),
        floor_id,
        thumbnail_path,
        root,
        target,
        progress,
        started_at: Instant::now(),
        warned_timeout: false,
    })
}

fn cleanup_genfloor_thumbnail_capture(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    capture: GenfloorThumbnailCapture,
) {
    commands.entity(capture.root).try_despawn();
    images.remove(capture.target.id());
}
