use std::collections::HashSet;
use std::f32::consts::TAU;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot, ScreenshotCaptured};
use serde::Serialize;

use crate::assets::SceneAssets;
use crate::object::registry::{
    ObjectLibrary, ObjectPartKind, PartAnimationDef, PartAnimationDriver,
};
use crate::object::visuals::{MaterialCache, PrimitiveMeshCache, VisualSpawnSettings};
use crate::types::{
    ActionClock, AnimationChannelsActive, AttackClock, BuildScene, ForcedAnimationChannel,
    LocomotionClock, ObjectPrefabId,
};

use super::state::{Gen3dDraft, Gen3dPreview};

const GEN3D_PREVIEW_EXPORT_FORMAT_VERSION: u32 = 2;
const GEN3D_PREVIEW_EXPORT_FRAME_COUNT: usize = 4;
const GEN3D_PREVIEW_EXPORT_CAPTURE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(120);
const GEN3D_PREVIEW_EXPORT_ANGLE_YAW_OFFSET: f32 = std::f32::consts::FRAC_PI_4; // 45 degrees.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gen3dPreviewExportPhase {
    Idle,
    Running,
    Completed,
    Failed,
}

impl Default for Gen3dPreviewExportPhase {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Gen3dPreviewExportStatus {
    pub(crate) phase: Gen3dPreviewExportPhase,
    pub(crate) run_id: Option<u64>,
    pub(crate) out_dir: Option<PathBuf>,
    pub(crate) manifest_path: Option<PathBuf>,
    pub(crate) total_channels: usize,
    pub(crate) completed_channels: usize,
    pub(crate) current_channel: Option<String>,
    pub(crate) message: String,
    pub(crate) error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Gen3dPreviewExportRequest {
    /// Parent directory for exports (a new `[ID]_[datetime]` folder is created inside).
    pub(crate) out_dir: Option<PathBuf>,
    pub(crate) channels: Vec<String>,
    /// Optional id used in the export folder name. When omitted, the exporter falls back to the
    /// export run id.
    pub(crate) export_id: Option<String>,
}

#[derive(Resource, Default)]
pub(crate) struct Gen3dPreviewExportRuntime {
    pub(crate) status: Gen3dPreviewExportStatus,
    next_run_id: u64,
    pending: Option<PendingGen3dPreviewExport>,
    active: Option<ActiveGen3dPreviewExport>,
}

impl Gen3dPreviewExportRuntime {
    pub(crate) fn is_running(&self) -> bool {
        self.pending.is_some() || self.active.is_some()
    }
}

#[derive(Clone, Debug)]
struct PendingGen3dPreviewExport {
    run_id: u64,
    out_dir: PathBuf,
    channels: Vec<PreviewExportChannelPlan>,
}

#[derive(Clone, Debug)]
struct PreviewExportChannelPlan {
    channel: String,
    file_stem: String,
    duration_secs: f32,
    finite: bool,
}

#[derive(Clone, Debug)]
struct PreviewExportAnglePlan {
    label: String,
    file_name: String,
    yaw: f32,
}

#[derive(Default)]
struct PreviewExportCaptureProgress {
    completed: usize,
}

#[derive(Clone, Debug)]
enum PendingPreviewCaptureKind {
    Angle { label: String },
    MotionFrame,
}

struct PendingPreviewCapture {
    kind: PendingPreviewCaptureKind,
    path: PathBuf,
    sample_secs: f32,
    expected_completed: usize,
    requested_at: Instant,
}

struct ActiveGen3dPreviewExport {
    run_id: u64,
    out_dir: PathBuf,
    temp_dir: PathBuf,
    target: Handle<Image>,
    camera: Entity,
    model_root: Entity,
    focus: Vec3,
    yaw: f32,
    pitch: f32,
    distance: f32,
    angle_channel: String,
    angles: Vec<PreviewExportAnglePlan>,
    angle_index: usize,
    channels: Vec<PreviewExportChannelPlan>,
    channel_index: usize,
    frame_paths: Vec<PathBuf>,
    pending_capture: Option<PendingPreviewCapture>,
    capture_progress: Arc<Mutex<PreviewExportCaptureProgress>>,
    manifest_angles: Vec<PreviewExportManifestAngleV1>,
    manifest_channels: Vec<PreviewExportManifestChannelV1>,
}

#[derive(Component)]
pub(crate) struct Gen3dPreviewExportModelRoot;

#[derive(Component)]
pub(crate) struct Gen3dPreviewExportCamera;

#[derive(Serialize)]
struct PreviewExportCameraManifestV1 {
    yaw: f32,
    pitch: f32,
    distance: f32,
}

#[derive(Clone, Debug, Serialize)]
struct PreviewExportManifestAngleV1 {
    index: usize,
    label: String,
    file: String,
    yaw: f32,
    pitch: f32,
    distance: f32,
    channel: String,
    sample_secs: f32,
}

#[derive(Clone, Debug, Serialize)]
struct PreviewExportManifestChannelV1 {
    index: usize,
    channel: String,
    still_file: String,
    gif_file: String,
    frame_count: usize,
    duration_secs: f32,
    finite: bool,
    still_sample_secs: f32,
}

#[derive(Serialize)]
struct PreviewExportManifestV1 {
    format_version: u32,
    run_id: u64,
    width_px: u32,
    height_px: u32,
    out_dir: String,
    camera: PreviewExportCameraManifestV1,
    angles: Vec<PreviewExportManifestAngleV1>,
    channels: Vec<PreviewExportManifestChannelV1>,
}

pub(crate) fn request_gen3d_preview_export(
    build_scene: &State<BuildScene>,
    draft: &Gen3dDraft,
    preview: &Gen3dPreview,
    library: &ObjectLibrary,
    runtime: &mut Gen3dPreviewExportRuntime,
    request: Gen3dPreviewExportRequest,
) -> Result<Gen3dPreviewExportStatus, String> {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return Err("Gen3D preview export requires BuildScene::Preview.".to_string());
    }
    if runtime.is_running() {
        return Err("Preview export already running.".to_string());
    }
    if preview.root.is_none() || preview.camera.is_none() {
        return Err("Gen3D preview scene is not ready yet.".to_string());
    }
    if draft.root_def().is_none() || draft.total_non_projectile_primitive_parts() == 0 {
        return Err("No Gen3D draft preview is available to export.".to_string());
    }

    let object_id = super::gen3d_draft_object_id();
    let available_channels = preview_export_available_channels(library, object_id);
    let requested_channels = normalize_requested_channels(request.channels);
    let channels = if requested_channels.is_empty() {
        available_channels.clone()
    } else {
        let available: HashSet<&str> = available_channels.iter().map(|v| v.as_str()).collect();
        let invalid: Vec<String> = requested_channels
            .iter()
            .filter(|channel| !available.contains(channel.as_str()))
            .cloned()
            .collect();
        if !invalid.is_empty() {
            return Err(format!(
                "Unknown preview channel(s): {}. Available: {}.",
                invalid.join(", "),
                available_channels.join(", ")
            ));
        }
        requested_channels
    };
    if channels.is_empty() {
        return Err("No preview channels are available to export.".to_string());
    }

    let run_id = runtime.next_run_id.max(1);
    runtime.next_run_id = run_id.saturating_add(1);
    let base_dir = request
        .out_dir
        .unwrap_or_else(default_preview_export_root_dir);
    let export_id = request
        .export_id
        .as_deref()
        .map(preview_export_normalize_folder_id)
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| run_id.to_string());
    let out_dir = base_dir.join(preview_export_folder_name(&export_id));
    let channel_plans = channels
        .into_iter()
        .enumerate()
        .map(|(index, channel)| {
            let (duration_secs, finite) =
                preview_export_channel_duration_secs(library, object_id, &channel)
                    .unwrap_or((1.0, false));
            let stem = format!("{:02}_{}", index + 1, preview_export_file_stem(&channel));
            PreviewExportChannelPlan {
                channel,
                file_stem: stem,
                duration_secs,
                finite,
            }
        })
        .collect::<Vec<_>>();

    let status = Gen3dPreviewExportStatus {
        phase: Gen3dPreviewExportPhase::Running,
        run_id: Some(run_id),
        out_dir: Some(out_dir.clone()),
        manifest_path: None,
        total_channels: channel_plans.len(),
        completed_channels: 0,
        current_channel: channel_plans.first().map(|plan| plan.channel.clone()),
        message: format!(
            "Queued preview export for {} channel(s).",
            channel_plans.len()
        ),
        error: None,
    };

    runtime.status = status.clone();
    runtime.pending = Some(PendingGen3dPreviewExport {
        run_id,
        out_dir,
        channels: channel_plans,
    });
    Ok(status)
}

pub(crate) fn gen3d_preview_export_status_payload(
    status: &Gen3dPreviewExportStatus,
) -> serde_json::Value {
    let phase = match status.phase {
        Gen3dPreviewExportPhase::Idle => "idle",
        Gen3dPreviewExportPhase::Running => "running",
        Gen3dPreviewExportPhase::Completed => "completed",
        Gen3dPreviewExportPhase::Failed => "failed",
    };

    serde_json::json!({
        "phase": phase,
        "run_id": status.run_id,
        "out_dir": status.out_dir.as_ref().map(|path| path.display().to_string()),
        "manifest_path": status.manifest_path.as_ref().map(|path| path.display().to_string()),
        "total_channels": status.total_channels,
        "completed_channels": status.completed_channels,
        "current_channel": status.current_channel,
        "message": status.message,
        "error": status.error,
    })
}

pub(crate) fn gen3d_preview_export_poll(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    assets: Res<SceneAssets>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut material_cache: ResMut<MaterialCache>,
    mut mesh_cache: ResMut<PrimitiveMeshCache>,
    mut library: ResMut<ObjectLibrary>,
    draft: Res<Gen3dDraft>,
    preview: Res<Gen3dPreview>,
    mut export_roots: Query<
        (
            &mut AnimationChannelsActive,
            &mut LocomotionClock,
            &mut AttackClock,
            &mut ActionClock,
            &mut ForcedAnimationChannel,
        ),
        With<Gen3dPreviewExportModelRoot>,
    >,
    mut runtime: ResMut<Gen3dPreviewExportRuntime>,
) {
    if !runtime.is_running() {
        return;
    }

    if preview.root.is_none() || preview.camera.is_none() {
        fail_preview_export(
            &mut commands,
            &mut images,
            &mut runtime,
            None,
            "Preview export canceled because the Gen3D preview scene is no longer available."
                .to_string(),
        );
        return;
    }

    if runtime.active.is_none() {
        let Some(pending) = runtime.pending.take() else {
            return;
        };

        match start_preview_export(
            &mut commands,
            &asset_server,
            &assets,
            &mut images,
            &mut meshes,
            &mut materials,
            &mut material_cache,
            &mut mesh_cache,
            &mut library,
            &draft,
            &preview,
            pending,
        ) {
            Ok(active) => {
                if let Some(first_angle) = active.angles.first() {
                    runtime.status.current_channel =
                        Some(format!("angle/{}", first_angle.label.as_str()));
                    runtime.status.message = format!("Exporting angle {}…", first_angle.label);
                } else if let Some(first_channel) = active.channels.first() {
                    runtime.status.current_channel = Some(first_channel.channel.clone());
                    runtime.status.message = format!(
                        "Exporting {} ({}/{})…",
                        first_channel.channel,
                        1,
                        active.channels.len()
                    );
                }
                runtime.active = Some(active);
                // The camera/model entities are spawned via Commands, so they are not available
                // to queries until the next frame after Commands have been applied.
                return;
            }
            Err(err) => fail_preview_export(&mut commands, &mut images, &mut runtime, None, err),
        }
    }

    let Some(mut active) = runtime.active.take() else {
        return;
    };

    if let Some(pending_capture) = active.pending_capture.as_ref() {
        let completed = active
            .capture_progress
            .lock()
            .map(|guard| guard.completed)
            .unwrap_or(0);
        if completed >= pending_capture.expected_completed {
            match &pending_capture.kind {
                PendingPreviewCaptureKind::Angle { .. } => {
                    let Some(plan) = active.angles.get(active.angle_index).cloned() else {
                        fail_preview_export(
                            &mut commands,
                            &mut images,
                            &mut runtime,
                            Some(active),
                            "Preview export angle plan is missing.".to_string(),
                        );
                        return;
                    };
                    active.manifest_angles.push(PreviewExportManifestAngleV1 {
                        index: active.manifest_angles.len().saturating_add(1),
                        label: plan.label.clone(),
                        file: plan.file_name.clone(),
                        yaw: plan.yaw,
                        pitch: active.pitch,
                        distance: active.distance,
                        channel: active.angle_channel.clone(),
                        sample_secs: pending_capture.sample_secs,
                    });
                    active.angle_index = active.angle_index.saturating_add(1);
                }
                PendingPreviewCaptureKind::MotionFrame => {
                    active.frame_paths.push(pending_capture.path.clone());
                }
            }
            active.pending_capture = None;
        } else if pending_capture.requested_at.elapsed() > GEN3D_PREVIEW_EXPORT_CAPTURE_TIMEOUT {
            let label = match &pending_capture.kind {
                PendingPreviewCaptureKind::Angle { label } => {
                    format!("angle `{}`", label.as_str())
                }
                PendingPreviewCaptureKind::MotionFrame => active
                    .channels
                    .get(active.channel_index)
                    .map(|plan| format!("channel `{}`", plan.channel.as_str()))
                    .unwrap_or_else(|| "channel".to_string()),
            };
            fail_preview_export(
                &mut commands,
                &mut images,
                &mut runtime,
                Some(active),
                format!("Timed out while capturing preview frame for {label}."),
            );
            return;
        } else {
            runtime.active = Some(active);
            return;
        }
    }

    if active.angle_index < active.angles.len() {
        let Some(plan) = active.angles.get(active.angle_index).cloned() else {
            fail_preview_export(
                &mut commands,
                &mut images,
                &mut runtime,
                Some(active),
                "Preview export angle plan is missing.".to_string(),
            );
            return;
        };

        let Ok((mut channels, mut locomotion, mut attack, mut action, mut forced)) =
            export_roots.get_mut(active.model_root)
        else {
            fail_preview_export(
                &mut commands,
                &mut images,
                &mut runtime,
                Some(active),
                "Preview export model is no longer available.".to_string(),
            );
            return;
        };

        apply_preview_export_sample(
            &library,
            super::gen3d_draft_object_id(),
            &active.angle_channel,
            0.0,
            &mut channels,
            &mut locomotion,
            &mut attack,
            &mut action,
            &mut forced,
        );

        runtime.status.current_channel = Some(format!("angle/{}", plan.label.as_str()));
        runtime.status.message = format!("Exporting angle {}…", plan.label);

        let angle_transform = crate::orbit_capture::orbit_transform(
            plan.yaw,
            active.pitch,
            active.distance,
            active.focus,
        );
        commands.entity(active.camera).insert(angle_transform);

        let path = active.out_dir.join(&plan.file_name);
        let expected_completed = active
            .capture_progress
            .lock()
            .map(|guard| guard.completed.saturating_add(1))
            .unwrap_or(1);
        let progress = active.capture_progress.clone();
        let path_for_capture = path.clone();
        commands
            .spawn(Screenshot::image(active.target.clone()))
            .observe(move |event: On<ScreenshotCaptured>| {
                if let Some(parent) = path_for_capture.parent() {
                    if let Err(err) = std::fs::create_dir_all(parent) {
                        error!(
                            "Gen3D preview export: failed to create directory {}: {err}",
                            parent.display()
                        );
                    }
                }
                let mut saver = save_to_disk(path_for_capture.clone());
                saver(event);
                if let Ok(mut guard) = progress.lock() {
                    guard.completed = guard.completed.saturating_add(1);
                }
            });

        active.pending_capture = Some(PendingPreviewCapture {
            kind: PendingPreviewCaptureKind::Angle {
                label: plan.label.clone(),
            },
            path,
            sample_secs: 0.0,
            expected_completed,
            requested_at: Instant::now(),
        });
        runtime.active = Some(active);
        return;
    }

    if active.frame_paths.len() >= GEN3D_PREVIEW_EXPORT_FRAME_COUNT {
        if let Err(err) = finalize_preview_export_channel(&mut active) {
            fail_preview_export(&mut commands, &mut images, &mut runtime, Some(active), err);
            return;
        }

        runtime.status.completed_channels = active.channel_index;
        if active.channel_index >= active.channels.len() {
            match finalize_preview_export_manifest(&active) {
                Ok(manifest_path) => {
                    runtime.status.phase = Gen3dPreviewExportPhase::Completed;
                    runtime.status.completed_channels = runtime.status.total_channels;
                    runtime.status.manifest_path = Some(manifest_path.clone());
                    runtime.status.current_channel = None;
                    runtime.status.message =
                        format!("Preview export completed: {}", manifest_path.display());
                    cleanup_active_preview_export(&mut commands, &mut images, Some(active));
                }
                Err(err) => {
                    fail_preview_export(&mut commands, &mut images, &mut runtime, Some(active), err)
                }
            }
            return;
        }

        runtime.status.current_channel = active
            .channels
            .get(active.channel_index)
            .map(|plan| plan.channel.clone());
        if let Some(current_channel) = runtime.status.current_channel.as_deref() {
            runtime.status.message = format!(
                "Exporting {} ({}/{})…",
                current_channel,
                active.channel_index + 1,
                active.channels.len()
            );
        }
    }

    let plan = active.channels[active.channel_index].clone();
    runtime.status.current_channel = Some(plan.channel.clone());
    runtime.status.message = format!(
        "Exporting {} ({}/{})…",
        plan.channel,
        active.channel_index + 1,
        active.channels.len()
    );
    let sample_index = active.frame_paths.len();
    let sample_secs =
        preview_export_sample_secs(&plan, sample_index, GEN3D_PREVIEW_EXPORT_FRAME_COUNT);
    let frame_path = active.temp_dir.join(format!(
        "{}_frame_{:02}.png",
        plan.file_stem,
        sample_index + 1
    ));

    let Ok((mut channels, mut locomotion, mut attack, mut action, mut forced)) =
        export_roots.get_mut(active.model_root)
    else {
        fail_preview_export(
            &mut commands,
            &mut images,
            &mut runtime,
            Some(active),
            "Preview export model is no longer available.".to_string(),
        );
        return;
    };

    apply_preview_export_sample(
        &library,
        super::gen3d_draft_object_id(),
        &plan.channel,
        sample_secs,
        &mut channels,
        &mut locomotion,
        &mut attack,
        &mut action,
        &mut forced,
    );

    let expected_completed = active
        .capture_progress
        .lock()
        .map(|guard| guard.completed.saturating_add(1))
        .unwrap_or(1);
    let progress = active.capture_progress.clone();
    let frame_path_for_capture = frame_path.clone();
    let front_transform = crate::orbit_capture::orbit_transform(
        active.yaw,
        active.pitch,
        active.distance,
        active.focus,
    );
    commands.entity(active.camera).insert(front_transform);
    commands
        .spawn(Screenshot::image(active.target.clone()))
        .observe(move |event: On<ScreenshotCaptured>| {
            if let Some(parent) = frame_path_for_capture.parent() {
                if let Err(err) = std::fs::create_dir_all(parent) {
                    error!(
                        "Gen3D preview export: failed to create directory {}: {err}",
                        parent.display()
                    );
                }
            }
            let mut saver = save_to_disk(frame_path_for_capture.clone());
            saver(event);
            if let Ok(mut guard) = progress.lock() {
                guard.completed = guard.completed.saturating_add(1);
            }
        });

    active.pending_capture = Some(PendingPreviewCapture {
        kind: PendingPreviewCaptureKind::MotionFrame,
        path: frame_path,
        sample_secs,
        expected_completed,
        requested_at: Instant::now(),
    });
    runtime.active = Some(active);
}

fn start_preview_export(
    commands: &mut Commands,
    asset_server: &AssetServer,
    assets: &SceneAssets,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    material_cache: &mut MaterialCache,
    mesh_cache: &mut PrimitiveMeshCache,
    library: &mut ObjectLibrary,
    draft: &Gen3dDraft,
    preview: &Gen3dPreview,
    pending: PendingGen3dPreviewExport,
) -> Result<ActiveGen3dPreviewExport, String> {
    let Some(preview_root) = preview.root else {
        return Err("Gen3D preview scene root is missing.".to_string());
    };

    fn finite_or(value: f32, default: f32) -> f32 {
        if value.is_finite() {
            value
        } else {
            default
        }
    }

    let focus = super::effective_preview_camera_focus(preview, None);
    let focus = if focus.is_finite() { focus } else { Vec3::ZERO };
    let yaw = finite_or(preview.yaw, super::GEN3D_PREVIEW_DEFAULT_YAW);
    let pitch = finite_or(preview.pitch, super::GEN3D_PREVIEW_DEFAULT_PITCH);
    let mut distance = finite_or(preview.distance, super::GEN3D_PREVIEW_DEFAULT_DISTANCE);
    if !distance.is_finite() || distance <= 1e-3 {
        distance = super::GEN3D_PREVIEW_DEFAULT_DISTANCE;
    }
    distance = distance.clamp(0.25, 100.0);
    let camera_transform = crate::orbit_capture::orbit_transform(yaw, pitch, distance, focus);

    let angle_channel = pending
        .channels
        .iter()
        .find(|plan| plan.channel.eq_ignore_ascii_case("idle"))
        .map(|plan| plan.channel.clone())
        .or_else(|| pending.channels.first().map(|plan| plan.channel.clone()))
        .unwrap_or_else(|| "idle".to_string());

    let angles = vec![
        PreviewExportAnglePlan {
            label: "front".to_string(),
            file_name: "angle_front.png".to_string(),
            yaw,
        },
        PreviewExportAnglePlan {
            label: "left_front".to_string(),
            file_name: "angle_left_front.png".to_string(),
            yaw: yaw - GEN3D_PREVIEW_EXPORT_ANGLE_YAW_OFFSET,
        },
        PreviewExportAnglePlan {
            label: "right_front".to_string(),
            file_name: "angle_right_front.png".to_string(),
            yaw: yaw + GEN3D_PREVIEW_EXPORT_ANGLE_YAW_OFFSET,
        },
    ];

    std::fs::create_dir_all(&pending.out_dir).map_err(|err| {
        format!(
            "Failed to create export directory {}: {err}",
            pending.out_dir.display()
        )
    })?;
    let temp_dir = pending.out_dir.join(".tmp_preview_frames");
    std::fs::create_dir_all(&temp_dir).map_err(|err| {
        format!(
            "Failed to create temp export directory {}: {err}",
            temp_dir.display()
        )
    })?;

    for def in draft.defs.iter().cloned() {
        library.upsert(def);
    }

    let target = crate::orbit_capture::create_render_target(
        images,
        super::GEN3D_PREVIEW_WIDTH_PX,
        super::GEN3D_PREVIEW_HEIGHT_PX,
    );

    let aspect =
        super::GEN3D_PREVIEW_WIDTH_PX.max(1) as f32 / super::GEN3D_PREVIEW_HEIGHT_PX.max(1) as f32;
    let mut projection = bevy::camera::PerspectiveProjection::default();
    projection.aspect_ratio = aspect;

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
            bevy::camera::visibility::RenderLayers::layer(super::GEN3D_PREVIEW_LAYER),
            camera_transform,
            Gen3dPreviewExportCamera,
        ))
        .id();
    commands.entity(preview_root).add_child(camera);

    let mut model_entity = commands.spawn((
        Transform::IDENTITY,
        Visibility::Inherited,
        Gen3dPreviewExportModelRoot,
        ObjectPrefabId(super::gen3d_draft_object_id()),
        ForcedAnimationChannel::default(),
        AnimationChannelsActive::default(),
        LocomotionClock {
            t: 0.0,
            distance_m: 0.0,
            signed_distance_m: 0.0,
            speed_mps: 0.0,
            last_translation: Vec3::ZERO,
            last_move_dir_xz: Vec2::ZERO,
        },
        AttackClock::default(),
        ActionClock::default(),
    ));
    crate::object::visuals::spawn_object_visuals_with_settings(
        &mut model_entity,
        library,
        asset_server,
        assets,
        meshes,
        materials,
        material_cache,
        mesh_cache,
        super::gen3d_draft_object_id(),
        None,
        VisualSpawnSettings {
            mark_parts: false,
            render_layer: Some(super::GEN3D_PREVIEW_LAYER),
        },
    );
    let model_root = model_entity.id();
    commands.entity(preview_root).add_child(model_root);

    Ok(ActiveGen3dPreviewExport {
        run_id: pending.run_id,
        out_dir: pending.out_dir,
        temp_dir,
        target,
        camera,
        model_root,
        focus,
        yaw,
        pitch,
        distance,
        angle_channel,
        angles,
        angle_index: 0,
        channels: pending.channels,
        channel_index: 0,
        frame_paths: Vec::new(),
        pending_capture: None,
        capture_progress: Arc::new(Mutex::new(PreviewExportCaptureProgress::default())),
        manifest_angles: Vec::new(),
        manifest_channels: Vec::new(),
    })
}

fn cleanup_active_preview_export(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    active: Option<ActiveGen3dPreviewExport>,
) {
    let Some(active) = active else {
        return;
    };
    let _ = std::fs::remove_dir_all(&active.temp_dir);
    commands.entity(active.camera).try_despawn();
    commands.entity(active.model_root).try_despawn();
    images.remove(active.target.id());
}

fn fail_preview_export(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    runtime: &mut Gen3dPreviewExportRuntime,
    active: Option<ActiveGen3dPreviewExport>,
    err: String,
) {
    let run_id = runtime.status.run_id.unwrap_or(0);
    let out_dir = runtime
        .status
        .out_dir
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let current_channel = runtime
        .status
        .current_channel
        .as_deref()
        .unwrap_or_default();
    error!(
        "Gen3D preview export failed (run_id={}, out_dir={}, current_channel={}): {}",
        run_id, out_dir, current_channel, err
    );

    runtime.pending = None;
    cleanup_active_preview_export(commands, images, active.or_else(|| runtime.active.take()));
    runtime.status.phase = Gen3dPreviewExportPhase::Failed;
    runtime.status.current_channel = None;
    runtime.status.error = Some(err.clone());
    runtime.status.message = format!("Preview export failed: {err}");
}

fn finalize_preview_export_channel(active: &mut ActiveGen3dPreviewExport) -> Result<(), String> {
    let channel_plan = active
        .channels
        .get(active.channel_index)
        .cloned()
        .ok_or_else(|| "Preview export channel index is out of range.".to_string())?;
    let still_index = active.frame_paths.len() / 2;
    let still_source = active
        .frame_paths
        .get(still_index)
        .ok_or_else(|| "Preview export did not capture a still frame.".to_string())?;
    let still_file = format!("{}_still.png", channel_plan.file_stem);
    let gif_file = format!("{}_anim.gif", channel_plan.file_stem);
    let still_path = active.out_dir.join(&still_file);
    let gif_path = active.out_dir.join(&gif_file);

    std::fs::copy(still_source, &still_path).map_err(|err| {
        format!(
            "Failed to write still image {}: {err}",
            still_path.display()
        )
    })?;
    write_preview_export_gif(
        &gif_path,
        &active.frame_paths,
        channel_plan.duration_secs,
        GEN3D_PREVIEW_EXPORT_FRAME_COUNT,
    )?;

    let still_sample_secs =
        preview_export_sample_secs(&channel_plan, still_index, GEN3D_PREVIEW_EXPORT_FRAME_COUNT);
    active
        .manifest_channels
        .push(PreviewExportManifestChannelV1 {
            index: active.channel_index + 1,
            channel: channel_plan.channel.clone(),
            still_file,
            gif_file,
            frame_count: GEN3D_PREVIEW_EXPORT_FRAME_COUNT,
            duration_secs: channel_plan.duration_secs,
            finite: channel_plan.finite,
            still_sample_secs,
        });

    for frame in active.frame_paths.drain(..) {
        let _ = std::fs::remove_file(frame);
    }
    active.channel_index = active.channel_index.saturating_add(1);
    Ok(())
}

fn finalize_preview_export_manifest(active: &ActiveGen3dPreviewExport) -> Result<PathBuf, String> {
    let manifest_path = active.out_dir.join("manifest.json");
    let doc = PreviewExportManifestV1 {
        format_version: GEN3D_PREVIEW_EXPORT_FORMAT_VERSION,
        run_id: active.run_id,
        width_px: super::GEN3D_PREVIEW_WIDTH_PX,
        height_px: super::GEN3D_PREVIEW_HEIGHT_PX,
        out_dir: active.out_dir.display().to_string(),
        camera: PreviewExportCameraManifestV1 {
            yaw: active.yaw,
            pitch: active.pitch,
            distance: active.distance,
        },
        angles: active.manifest_angles.clone(),
        channels: active.manifest_channels.clone(),
    };
    write_json_atomic(&manifest_path, &doc)?;
    Ok(manifest_path)
}

fn write_preview_export_gif(
    path: &Path,
    frames: &[PathBuf],
    duration_secs: f32,
    frame_count: usize,
) -> Result<(), String> {
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Frame};

    if frames.is_empty() {
        return Err("Preview export captured no GIF frames.".to_string());
    }

    let delay_ms = ((duration_secs.max(0.2) / frame_count.max(1) as f32) * 1000.0)
        .round()
        .clamp(40.0, 2_000.0) as u32;
    let file = std::fs::File::create(path)
        .map_err(|err| format!("Failed to create GIF {}: {err}", path.display()))?;
    let mut encoder = GifEncoder::new(file);
    encoder
        .set_repeat(Repeat::Infinite)
        .map_err(|err| format!("Failed to configure GIF repeat {}: {err}", path.display()))?;

    let mut expected_dims = None;
    for frame_path in frames {
        let rgba = image::open(frame_path)
            .map_err(|err| format!("Failed to read frame {}: {err}", frame_path.display()))?
            .to_rgba8();
        let dims = rgba.dimensions();
        if let Some(expected_dims) = expected_dims {
            if dims != expected_dims {
                return Err(format!(
                    "GIF frame size mismatch: expected {:?}, got {:?} for {}.",
                    expected_dims,
                    dims,
                    frame_path.display()
                ));
            }
        } else {
            expected_dims = Some(dims);
        }
        encoder
            .encode_frame(Frame::from_parts(
                rgba,
                0,
                0,
                Delay::from_numer_denom_ms(delay_ms, 1),
            ))
            .map_err(|err| format!("Failed to encode GIF {}: {err}", path.display()))?;
    }
    Ok(())
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let Some(parent) = path.parent() else {
        return Err(format!("No parent directory for {}.", path.display()));
    };
    std::fs::create_dir_all(parent)
        .map_err(|err| format!("Failed to create directory {}: {err}", parent.display()))?;
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|err| format!("Failed to serialize JSON {}: {err}", path.display()))?;
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, &bytes)
        .map_err(|err| format!("Failed to write {}: {err}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, path)
        .map_err(|err| format!("Failed to rename {}: {err}", path.display()))?;
    Ok(())
}

fn normalize_requested_channels(channels: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::<String>::new();
    let mut out = Vec::new();
    for channel in channels {
        let trimmed = channel.trim();
        if trimmed.is_empty() {
            continue;
        }
        let trimmed = trimmed.to_string();
        if seen.insert(trimmed.clone()) {
            out.push(trimmed);
        }
    }
    out
}

fn preview_export_available_channels(library: &ObjectLibrary, object_id: u128) -> Vec<String> {
    let mut channels = library.animation_channels_ordered(object_id);
    if channels.is_empty() {
        channels.push("idle".to_string());
    }
    channels
}

fn preview_export_channel_duration_secs(
    library: &ObjectLibrary,
    object_id: u128,
    channel: &str,
) -> Option<(f32, bool)> {
    fn visit(
        library: &ObjectLibrary,
        object_id: u128,
        channel: &str,
        move_speed_mps: f32,
        visited: &mut HashSet<u128>,
        best: &mut Option<f32>,
        finite: &mut bool,
    ) {
        if !visited.insert(object_id) {
            return;
        }
        let Some(def) = library.get(object_id) else {
            return;
        };

        for part in &def.parts {
            for slot in &part.animations {
                if slot.channel.as_ref() != channel {
                    continue;
                }

                if matches!(
                    slot.spec.driver,
                    PartAnimationDriver::AttackTime | PartAnimationDriver::ActionTime
                ) || matches!(slot.spec.clip, PartAnimationDef::Once { .. })
                {
                    *finite = true;
                }

                let base_units = match &slot.spec.clip {
                    PartAnimationDef::Loop { duration_secs, .. }
                    | PartAnimationDef::Once { duration_secs, .. } => duration_secs.abs(),
                    PartAnimationDef::PingPong { duration_secs, .. } => duration_secs.abs() * 2.0,
                    PartAnimationDef::Spin {
                        radians_per_unit, ..
                    } => {
                        let radians_per_unit = radians_per_unit.abs();
                        if radians_per_unit <= 1e-4 {
                            continue;
                        }
                        TAU / radians_per_unit
                    }
                };

                let speed_scale = slot.spec.speed_scale.abs().max(1e-3);
                let units = base_units / speed_scale;
                let secs = match slot.spec.driver {
                    PartAnimationDriver::Always
                    | PartAnimationDriver::AttackTime
                    | PartAnimationDriver::ActionTime => units,
                    PartAnimationDriver::MovePhase | PartAnimationDriver::MoveDistance => {
                        units / move_speed_mps.max(0.25)
                    }
                };
                if secs.is_finite() && secs > 1e-3 {
                    *best = Some(best.map_or(secs, |current| current.max(secs)));
                }
            }

            if let ObjectPartKind::ObjectRef { object_id: child } = &part.kind {
                visit(
                    library,
                    *child,
                    channel,
                    move_speed_mps,
                    visited,
                    best,
                    finite,
                );
            }
        }
    }

    let move_speed_mps = library
        .mobility(object_id)
        .map(|mobility| mobility.max_speed.abs())
        .filter(|value| value.is_finite())
        .unwrap_or(1.0)
        .max(0.25);

    let mut visited = HashSet::new();
    let mut best = None;
    let mut finite = false;
    visit(
        library,
        object_id,
        channel,
        move_speed_mps,
        &mut visited,
        &mut best,
        &mut finite,
    );

    if channel == "attack" {
        if let Some(attack_secs) = library.channel_attack_duration_secs(object_id, channel) {
            best = Some(best.map_or(attack_secs, |current| current.max(attack_secs)));
            finite = true;
        }
    }
    if library
        .channel_action_duration_secs(object_id, channel)
        .is_some()
    {
        finite = true;
    }

    best.map(|secs| (secs.clamp(0.1, 12.0), finite))
}

fn preview_export_sample_secs(
    plan: &PreviewExportChannelPlan,
    sample_index: usize,
    frame_count: usize,
) -> f32 {
    let duration_secs = plan.duration_secs.max(0.1);
    if frame_count <= 1 {
        return 0.0;
    }

    let ratio = if plan.finite {
        sample_index as f32 / (frame_count - 1) as f32
    } else {
        sample_index as f32 / frame_count as f32
    };
    (duration_secs * ratio).clamp(0.0, duration_secs)
}

fn apply_preview_export_sample(
    library: &ObjectLibrary,
    object_id: u128,
    channel: &str,
    sample_secs: f32,
    channels: &mut AnimationChannelsActive,
    locomotion: &mut LocomotionClock,
    attack: &mut AttackClock,
    action: &mut ActionClock,
    forced: &mut ForcedAnimationChannel,
) {
    let sample_secs = sample_secs.max(0.0);
    let wants_move = channel == "move" || library.channel_uses_move_driver(object_id, channel);
    let wants_attack = library
        .channel_attack_duration_secs(object_id, channel)
        .is_some();
    let wants_action = channel == "action"
        || library
            .channel_action_duration_secs(object_id, channel)
            .is_some();

    channels.moving = wants_move;
    channels.attacking_primary = wants_attack;
    channels.acting = wants_action;
    forced.channel = channel.to_string();

    let speed_mps = library
        .mobility(object_id)
        .map(|mobility| mobility.max_speed.abs())
        .filter(|value| value.is_finite())
        .unwrap_or(1.0)
        .max(0.25);

    locomotion.speed_mps = if wants_move { speed_mps } else { 0.0 };
    locomotion.t = if wants_move {
        sample_secs * speed_mps
    } else {
        0.0
    };
    locomotion.distance_m = if wants_move {
        sample_secs * speed_mps
    } else {
        0.0
    };
    locomotion.signed_distance_m = if wants_move {
        sample_secs * speed_mps
    } else {
        0.0
    };
    locomotion.last_translation = Vec3::ZERO;
    locomotion.last_move_dir_xz = if wants_move { Vec2::Y } else { Vec2::ZERO };

    if let Some(duration_secs) = library.channel_attack_duration_secs(object_id, channel) {
        attack.started_at_secs = 0.0;
        attack.duration_secs = duration_secs;
    } else {
        attack.started_at_secs = 0.0;
        attack.duration_secs = 0.0;
    }

    if let Some(duration_secs) = library.channel_action_duration_secs(object_id, channel) {
        action.started_at_secs = 0.0;
        action.duration_secs = duration_secs;
    } else {
        action.started_at_secs = 0.0;
        action.duration_secs = 0.0;
    }
}

fn default_preview_export_root_dir() -> PathBuf {
    crate::paths::default_cache_dir().join("gen3d_preview_exports")
}

fn preview_export_normalize_folder_id(id: &str) -> String {
    let mut out = String::new();
    let mut last_was_sep = false;
    for ch in id.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            out.push(ch);
            last_was_sep = false;
        } else if !last_was_sep {
            out.push('_');
            last_was_sep = true;
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "export".to_string()
    } else {
        trimmed.to_string()
    }
}

fn preview_export_folder_name(id: &str) -> String {
    let id = preview_export_normalize_folder_id(id);
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    let date = now.date();
    let time = now.time();
    let datetime = format!(
        "{:04}{:02}{:02}_{:02}{:02}{:02}",
        date.year(),
        date.month() as u8,
        date.day(),
        time.hour(),
        time.minute(),
        time.second()
    );
    format!("{id}_{datetime}")
}

fn preview_export_file_stem(channel: &str) -> String {
    let mut out = String::new();
    let mut last_was_sep = false;
    for ch in channel.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_was_sep = false;
        } else if !last_was_sep {
            out.push('_');
            last_was_sep = true;
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "idle".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::registry::{
        ColliderProfile, MeshKey, ObjectDef, ObjectInteraction, ObjectPartDef, PartAnimationFamily,
        PartAnimationKeyframeDef, PartAnimationSpec, PartAnimationSpinAxisSpace,
        PrimitiveVisualDef,
    };

    fn build_test_library(channel: &str, spec: PartAnimationSpec) -> ObjectLibrary {
        let mut library = ObjectLibrary::default();
        library.upsert(ObjectDef {
            object_id: super::super::gen3d_draft_object_id(),
            label: "draft".into(),
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
                    color: Color::WHITE,
                    unlit: false,
                },
                Transform::IDENTITY,
            )
            .with_animation_slot(channel.to_string(), PartAnimationFamily::Base, spec)],
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
    fn preview_export_file_stem_sanitizes_channel_names() {
        assert_eq!(preview_export_file_stem("Idle"), "idle");
        assert_eq!(preview_export_file_stem("Left Slash!"), "left_slash");
        assert_eq!(preview_export_file_stem("   "), "idle");
    }

    #[test]
    fn preview_export_folder_id_sanitizes_and_keeps_single_separators() {
        assert_eq!(
            preview_export_normalize_folder_id("  Alpha Beta  "),
            "Alpha_Beta"
        );
        assert_eq!(preview_export_normalize_folder_id("a///b***c"), "a_b_c");
        assert_eq!(preview_export_normalize_folder_id("   "), "export");
    }

    #[test]
    fn preview_export_folder_name_uses_normalized_prefix() {
        let folder = preview_export_folder_name(" Sword Slash ");
        assert!(folder.starts_with("Sword_Slash_"), "folder={folder}");
        assert_eq!(folder.len(), "Sword_Slash_YYYYMMDD_HHMMSS".len());
    }

    #[test]
    fn preview_export_sample_secs_uses_closed_range_for_finite_channels() {
        let plan = PreviewExportChannelPlan {
            channel: "attack".into(),
            file_stem: "01_attack".into(),
            duration_secs: 1.2,
            finite: true,
        };
        assert_eq!(
            preview_export_sample_secs(
                &plan,
                GEN3D_PREVIEW_EXPORT_FRAME_COUNT - 1,
                GEN3D_PREVIEW_EXPORT_FRAME_COUNT
            ),
            1.2
        );
    }

    #[test]
    fn preview_export_duration_handles_spin_periods() {
        let library = build_test_library(
            "ambient",
            PartAnimationSpec {
                driver: PartAnimationDriver::Always,
                speed_scale: 1.0,
                time_offset_units: 0.0,
                basis: Transform::IDENTITY,
                clip: PartAnimationDef::Spin {
                    axis: Vec3::Y,
                    radians_per_unit: TAU,
                    axis_space: PartAnimationSpinAxisSpace::Join,
                },
            },
        );

        let (duration_secs, finite) = preview_export_channel_duration_secs(
            &library,
            super::super::gen3d_draft_object_id(),
            "ambient",
        )
        .expect("ambient spin duration");
        assert!((duration_secs - 1.0).abs() < 1e-4);
        assert!(!finite);
    }

    #[test]
    fn preview_export_duration_marks_once_clips_as_finite() {
        let library = build_test_library(
            "pose",
            PartAnimationSpec {
                driver: PartAnimationDriver::Always,
                speed_scale: 1.0,
                time_offset_units: 0.0,
                basis: Transform::IDENTITY,
                clip: PartAnimationDef::Once {
                    duration_secs: 0.8,
                    keyframes: vec![PartAnimationKeyframeDef {
                        time_secs: 0.0,
                        delta: Transform::IDENTITY,
                    }],
                },
            },
        );

        let (duration_secs, finite) = preview_export_channel_duration_secs(
            &library,
            super::super::gen3d_draft_object_id(),
            "pose",
        )
        .expect("pose duration");
        assert!((duration_secs - 0.8).abs() < 1e-4);
        assert!(finite);
    }
}
