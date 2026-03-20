use bevy::ecs::message::{MessageReader, MessageWriter};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use prost::{Enumeration, Message, Oneof};
use std::collections::{HashSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};

use crate::assets::SceneAssets;
use crate::config::AppConfig;
use crate::constants::*;
use crate::object::registry::{
    AnchorDef, AnchorRef, AttachmentDef, ColliderProfile, MaterialKey, MeleeAttackProfile, MeshKey,
    MobilityDef, MobilityMode, MovementBlockRule, ObjectDef, ObjectInteraction, ObjectLibrary,
    ObjectPartDef, ObjectPartKind, PartAnimationDef, PartAnimationDriver, PartAnimationKeyframeDef,
    PartAnimationSlot, PartAnimationSpec, PrimitiveParams, PrimitiveVisualDef,
    ProjectileObstacleRule, ProjectileProfile, RangedAttackProfile, UnitAttackKind,
    UnitAttackProfile,
};
use crate::object::visuals;
use crate::types::*;
use crate::workspace_ui::{
    PendingWorkspaceSwitch, WorkspaceCameraSnapshot, WorkspaceCameraState, WorkspaceTab,
};

const SCENE_DAT_VERSION: u32 = 9;
// Persist positions in centimeters (stable and easy to reason about for AI-authored worlds).
const DEFAULT_UNITS_PER_METER: u32 = 100;
const SCENE_AUTOSAVE_INTERVAL_SECS: f32 = 60.0;

pub(crate) fn ensure_default_scene_dat_exists(
    realm_id: &str,
    scene_id: &str,
) -> Result<(), String> {
    if realm_id.trim().is_empty() || scene_id.trim().is_empty() {
        return Err("realm_id/scene_id must be non-empty".to_string());
    }

    let path = crate::paths::scene_dat_path(realm_id, scene_id);
    if path.exists() {
        return Ok(());
    }

    let hero_prefab = crate::object::types::characters::hero::object_id();
    let player_start = Vec3::new(0.0, PLAYER_Y, 0.0);
    let instance_id = ObjectId::new_v4();
    let scene = SceneDat {
        version: SCENE_DAT_VERSION,
        units_per_meter: DEFAULT_UNITS_PER_METER,
        defs: Vec::new(),
        instances: vec![SceneDatObjectInstance {
            instance_id: Some(u128_to_uuid(instance_id.0)),
            base_object_id: Some(u128_to_uuid(hero_prefab)),
            x_units: quantize_world(player_start.x, DEFAULT_UNITS_PER_METER),
            y_units: quantize_world(player_start.y, DEFAULT_UNITS_PER_METER),
            z_units: quantize_world(player_start.z, DEFAULT_UNITS_PER_METER),
            rot_x: 0.0,
            rot_y: 0.0,
            rot_z: 0.0,
            rot_w: 1.0,
            tint: None,
            scale_x: None,
            scale_y: None,
            scale_z: None,
            forms: vec![u128_to_uuid(hero_prefab)],
            active_form: 0,
            is_protagonist: true,
        }],
    };
    let bytes = scene.encode_to_vec();
    write_atomic(&path, &bytes)
        .map_err(|err| format!("Failed to write {}: {err}", path.display()))?;
    Ok(())
}

#[derive(bevy::ecs::message::Message, Debug, Clone, Copy)]
pub(crate) struct SceneSaveRequest {
    pub(crate) reason: &'static str,
}

impl SceneSaveRequest {
    pub(crate) const fn new(reason: &'static str) -> Self {
        Self { reason }
    }
}

#[derive(Resource, Debug)]
pub(crate) struct SceneAutosaveState {
    timer: Timer,
    dirty: bool,
    primed: bool,
}

impl Default for SceneAutosaveState {
    fn default() -> Self {
        Self {
            timer: Timer::from_seconds(SCENE_AUTOSAVE_INTERVAL_SECS, TimerMode::Repeating),
            dirty: false,
            primed: false,
        }
    }
}

fn snapshot_camera(
    zoom: &CameraZoom,
    yaw: &CameraYaw,
    pitch: &CameraPitch,
    focus: &CameraFocus,
) -> WorkspaceCameraSnapshot {
    WorkspaceCameraSnapshot {
        zoom_t: zoom.t,
        yaw: yaw.yaw,
        yaw_initialized: yaw.initialized,
        pitch: pitch.pitch,
        focus: focus.position,
        focus_initialized: focus.initialized,
    }
}

fn restore_camera(
    snapshot: WorkspaceCameraSnapshot,
    zoom: &mut CameraZoom,
    yaw: &mut CameraYaw,
    pitch: &mut CameraPitch,
    focus: &mut CameraFocus,
) {
    zoom.t = snapshot.zoom_t;
    yaw.yaw = snapshot.yaw;
    yaw.initialized = snapshot.yaw_initialized;
    pitch.pitch = snapshot.pitch;
    focus.position = snapshot.focus;
    focus.initialized = snapshot.focus_initialized;
}

#[derive(SystemParam)]
pub(crate) struct WorkspaceSwitchDeps<'w> {
    config: Res<'w, AppConfig>,
    active: Res<'w, crate::realm::ActiveRealmScene>,
    asset_server: Res<'w, AssetServer>,
    assets: Res<'w, SceneAssets>,
    meshes: ResMut<'w, Assets<Mesh>>,
    materials: ResMut<'w, Assets<StandardMaterial>>,
    material_cache: ResMut<'w, crate::object::visuals::MaterialCache>,
    mesh_cache: ResMut<'w, crate::object::visuals::PrimitiveMeshCache>,
    library: ResMut<'w, ObjectLibrary>,
    camera_zoom: ResMut<'w, CameraZoom>,
    camera_yaw: ResMut<'w, CameraYaw>,
    camera_pitch: ResMut<'w, CameraPitch>,
    camera_focus: ResMut<'w, CameraFocus>,
    workspace_camera: ResMut<'w, WorkspaceCameraState>,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDat {
    #[prost(uint32, tag = "1")]
    version: u32,
    #[prost(uint32, tag = "2")]
    units_per_meter: u32,
    #[prost(message, repeated, tag = "3")]
    defs: Vec<SceneDatObjectDef>,
    #[prost(message, repeated, tag = "4")]
    instances: Vec<SceneDatObjectInstance>,
}

#[derive(Clone, PartialEq, Message)]
struct Uuid128Dat {
    #[prost(fixed64, tag = "1")]
    hi: u64,
    #[prost(fixed64, tag = "2")]
    lo: u64,
}

#[derive(Clone, PartialEq, Message)]
struct Float32Dat {
    #[prost(float, tag = "1")]
    value: f32,
}

#[derive(Clone, PartialEq, Message)]
struct ColorDat {
    #[prost(fixed32, tag = "1")]
    rgba: u32,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatObjectInstance {
    #[prost(message, optional, tag = "1")]
    instance_id: Option<Uuid128Dat>,
    #[prost(message, optional, tag = "2")]
    base_object_id: Option<Uuid128Dat>,
    #[prost(int32, tag = "3")]
    x_units: i32,
    #[prost(int32, tag = "4")]
    y_units: i32,
    #[prost(int32, tag = "5")]
    z_units: i32,
    #[prost(float, tag = "6")]
    rot_x: f32,
    #[prost(float, tag = "7")]
    rot_y: f32,
    #[prost(float, tag = "8")]
    rot_z: f32,
    #[prost(float, tag = "9")]
    rot_w: f32,
    #[prost(message, optional, tag = "10")]
    tint: Option<ColorDat>,
    #[prost(message, optional, tag = "11")]
    scale_x: Option<Float32Dat>,
    #[prost(message, optional, tag = "12")]
    scale_y: Option<Float32Dat>,
    #[prost(message, optional, tag = "13")]
    scale_z: Option<Float32Dat>,
    #[prost(message, repeated, tag = "14")]
    forms: Vec<Uuid128Dat>,
    #[prost(uint32, tag = "15")]
    active_form: u32,
    #[prost(bool, tag = "19")]
    is_protagonist: bool,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatObjectDef {
    #[prost(message, optional, tag = "1")]
    object_id: Option<Uuid128Dat>,
    #[prost(string, tag = "2")]
    label: String,
    #[prost(float, tag = "3")]
    size_x: f32,
    #[prost(float, tag = "4")]
    size_y: f32,
    #[prost(float, tag = "5")]
    size_z: f32,
    #[prost(message, optional, tag = "6")]
    collider: Option<SceneDatCollider>,
    #[prost(message, optional, tag = "7")]
    interaction: Option<SceneDatInteraction>,
    #[prost(message, repeated, tag = "8")]
    parts: Vec<SceneDatPartDef>,
    #[prost(message, optional, tag = "9")]
    minimap_color: Option<ColorDat>,
    #[prost(message, optional, tag = "10")]
    health_bar_offset_y: Option<Float32Dat>,
    #[prost(message, repeated, tag = "11")]
    anchors: Vec<SceneDatAnchorDef>,
    #[prost(message, optional, tag = "12")]
    mobility: Option<SceneDatMobility>,
    #[prost(message, optional, tag = "13")]
    projectile: Option<SceneDatProjectile>,
    #[prost(message, optional, tag = "14")]
    attack: Option<SceneDatUnitAttack>,
    #[prost(message, optional, tag = "15")]
    aim: Option<SceneDatAimProfile>,
    #[prost(message, optional, tag = "16")]
    ground_origin_y: Option<Float32Dat>,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatAimProfile {
    #[prost(message, optional, tag = "1")]
    max_yaw_delta_degrees: Option<Float32Dat>,
    #[prost(message, repeated, tag = "2")]
    components: Vec<Uuid128Dat>,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatMobility {
    #[prost(enumeration = "SceneDatMobilityMode", tag = "1")]
    mode: i32,
    #[prost(float, tag = "2")]
    max_speed: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Enumeration)]
enum SceneDatMobilityMode {
    Ground = 0,
    Air = 1,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatProjectile {
    #[prost(enumeration = "SceneDatProjectileObstacleRule", tag = "1")]
    obstacle_rule: i32,
    #[prost(float, tag = "2")]
    speed: f32,
    #[prost(float, tag = "3")]
    ttl_secs: f32,
    #[prost(int32, tag = "4")]
    damage: i32,
    #[prost(bool, tag = "5")]
    spawn_energy_impact: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Enumeration)]
enum SceneDatProjectileObstacleRule {
    BulletsBlockers = 0,
    LaserBlockers = 1,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatUnitAttack {
    #[prost(enumeration = "SceneDatUnitAttackKind", tag = "1")]
    kind: i32,
    #[prost(float, tag = "2")]
    cooldown_secs: f32,
    #[prost(int32, tag = "3")]
    damage: i32,
    #[prost(float, tag = "4")]
    anim_window_secs: f32,
    #[prost(message, optional, tag = "5")]
    melee: Option<SceneDatMeleeAttack>,
    #[prost(message, optional, tag = "6")]
    ranged: Option<SceneDatRangedAttack>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Enumeration)]
enum SceneDatUnitAttackKind {
    Unknown = 0,
    Melee = 1,
    RangedProjectile = 2,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatMeleeAttack {
    #[prost(float, tag = "1")]
    range: f32,
    #[prost(float, tag = "2")]
    radius: f32,
    #[prost(float, tag = "3")]
    arc_degrees: f32,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatRangedAttack {
    #[prost(message, optional, tag = "1")]
    projectile_prefab: Option<Uuid128Dat>,
    #[prost(message, optional, tag = "2")]
    muzzle: Option<SceneDatAnchorRef>,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatAnchorRef {
    #[prost(message, optional, tag = "1")]
    object_id: Option<Uuid128Dat>,
    #[prost(string, tag = "2")]
    anchor: String,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatAnchorDef {
    #[prost(string, tag = "1")]
    name: String,
    #[prost(message, optional, tag = "2")]
    transform: Option<SceneDatTransform>,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatCollider {
    #[prost(oneof = "scene_dat_collider::Kind", tags = "1, 2, 3")]
    kind: Option<scene_dat_collider::Kind>,
}

mod scene_dat_collider {
    use super::*;

    #[derive(Clone, PartialEq, Oneof)]
    pub enum Kind {
        #[prost(message, tag = "1")]
        None(EmptyDat),
        #[prost(message, tag = "2")]
        CircleXz(SceneDatCircleXz),
        #[prost(message, tag = "3")]
        AabbXz(SceneDatAabbXz),
    }
}

#[derive(Clone, PartialEq, Message)]
struct EmptyDat {}

#[derive(Clone, PartialEq, Message)]
struct SceneDatCircleXz {
    #[prost(float, tag = "1")]
    radius: f32,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatAabbXz {
    #[prost(float, tag = "1")]
    half_x: f32,
    #[prost(float, tag = "2")]
    half_z: f32,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatInteraction {
    #[prost(bool, tag = "1")]
    blocks_bullets: bool,
    #[prost(bool, tag = "2")]
    blocks_laser: bool,
    #[prost(message, optional, tag = "3")]
    movement_block: Option<SceneDatMovementBlock>,
    #[prost(bool, tag = "4")]
    supports_standing: bool,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatMovementBlock {
    #[prost(oneof = "scene_dat_movement_block::Kind", tags = "1, 2")]
    kind: Option<scene_dat_movement_block::Kind>,
}

mod scene_dat_movement_block {
    use super::*;

    #[derive(Clone, PartialEq, Oneof)]
    pub enum Kind {
        #[prost(message, tag = "1")]
        Always(EmptyDat),
        #[prost(message, tag = "2")]
        UpperBodyFraction(Float32Dat),
    }
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatPartDef {
    #[prost(message, optional, tag = "1")]
    part_id: Option<Uuid128Dat>,
    #[prost(message, optional, tag = "2")]
    transform: Option<SceneDatTransform>,
    #[prost(oneof = "scene_dat_part_def::Kind", tags = "3, 4, 5")]
    kind: Option<scene_dat_part_def::Kind>,
    #[prost(message, optional, tag = "6")]
    attachment: Option<SceneDatAttachment>,
    #[prost(message, repeated, tag = "7")]
    animations: Vec<SceneDatPartAnimationSlot>,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatAttachment {
    #[prost(string, tag = "1")]
    parent_anchor: String,
    #[prost(string, tag = "2")]
    child_anchor: String,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatPartAnimationSlot {
    #[prost(string, tag = "1")]
    channel: String,
    #[prost(message, optional, tag = "2")]
    animation: Option<SceneDatPartAnimation>,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatPartAnimation {
    // NOTE: keep tags disjoint from the non-oneof fields below (driver/speed/time_offset).
    #[prost(oneof = "scene_dat_part_animation::Kind", tags = "1, 2, 6, 7")]
    kind: Option<scene_dat_part_animation::Kind>,
    #[prost(enumeration = "SceneDatPartAnimationDriver", tag = "3")]
    driver: i32,
    #[prost(float, tag = "4")]
    speed_scale: f32,
    #[prost(float, tag = "5")]
    time_offset_units: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Enumeration)]
enum SceneDatPartAnimationDriver {
    Always = 0,
    MovePhase = 1,
    MoveDistance = 2,
    AttackTime = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Enumeration)]
enum SceneDatPartAnimationSpinAxisSpace {
    Join = 0,
    ChildLocal = 1,
}

mod scene_dat_part_animation {
    use super::*;

    #[derive(Clone, PartialEq, Oneof)]
    pub enum Kind {
        #[prost(message, tag = "1")]
        Loop(SceneDatPartAnimationLoop),
        #[prost(message, tag = "2")]
        Spin(SceneDatPartAnimationSpin),
        #[prost(message, tag = "6")]
        Once(SceneDatPartAnimationLoop),
        #[prost(message, tag = "7")]
        PingPong(SceneDatPartAnimationLoop),
    }
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatPartAnimationLoop {
    #[prost(float, tag = "1")]
    duration_secs: f32,
    #[prost(message, repeated, tag = "2")]
    keyframes: Vec<SceneDatPartAnimationKeyframe>,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatPartAnimationSpin {
    #[prost(float, tag = "1")]
    axis_x: f32,
    #[prost(float, tag = "2")]
    axis_y: f32,
    #[prost(float, tag = "3")]
    axis_z: f32,
    #[prost(float, tag = "4")]
    radians_per_unit: f32,
    #[prost(enumeration = "SceneDatPartAnimationSpinAxisSpace", tag = "5")]
    axis_space: i32,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatPartAnimationKeyframe {
    #[prost(float, tag = "1")]
    time_secs: f32,
    #[prost(message, optional, tag = "2")]
    delta: Option<SceneDatTransform>,
}

mod scene_dat_part_def {
    use super::*;

    #[derive(Clone, PartialEq, Oneof)]
    pub enum Kind {
        #[prost(message, tag = "3")]
        ObjectRef(Uuid128Dat),
        #[prost(message, tag = "4")]
        Primitive(SceneDatPrimitive),
        #[prost(string, tag = "5")]
        Model(String),
    }
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatTransform {
    #[prost(float, tag = "1")]
    tx: f32,
    #[prost(float, tag = "2")]
    ty: f32,
    #[prost(float, tag = "3")]
    tz: f32,
    #[prost(float, tag = "4")]
    rx: f32,
    #[prost(float, tag = "5")]
    ry: f32,
    #[prost(float, tag = "6")]
    rz: f32,
    #[prost(float, tag = "7")]
    rw: f32,
    #[prost(float, tag = "8")]
    sx: f32,
    #[prost(float, tag = "9")]
    sy: f32,
    #[prost(float, tag = "10")]
    sz: f32,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatPrimitive {
    #[prost(oneof = "scene_dat_primitive::Kind", tags = "1, 2")]
    kind: Option<scene_dat_primitive::Kind>,
}

mod scene_dat_primitive {
    use super::*;

    #[derive(Clone, PartialEq, Oneof)]
    pub enum Kind {
        #[prost(message, tag = "1")]
        MeshRef(SceneDatPrimitiveMeshRef),
        #[prost(message, tag = "2")]
        Solid(SceneDatPrimitiveSolid),
    }
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatPrimitiveMeshRef {
    #[prost(enumeration = "SceneDatMeshKey", tag = "1")]
    mesh: i32,
    #[prost(message, optional, tag = "2")]
    material: Option<SceneDatMaterialKey>,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatPrimitiveSolid {
    #[prost(enumeration = "SceneDatMeshKey", tag = "1")]
    mesh: i32,
    #[prost(message, optional, tag = "2")]
    params: Option<SceneDatPrimitiveParams>,
    #[prost(message, optional, tag = "3")]
    color: Option<ColorDat>,
    #[prost(bool, tag = "4")]
    unlit: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Enumeration)]
#[repr(i32)]
enum SceneDatMeshKey {
    UnitCube = 0,
    UnitCylinder = 1,
    UnitCone = 2,
    UnitSphere = 3,
    UnitPlane = 4,
    UnitCapsule = 5,
    UnitConicalFrustum = 6,
    UnitTorus = 7,
    UnitTriangle = 8,
    UnitTetrahedron = 9,
    TreeTrunk = 10,
    TreeCone = 11,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatMaterialKey {
    #[prost(oneof = "scene_dat_material_key::Kind", tags = "1, 2, 3, 4, 5, 6")]
    kind: Option<scene_dat_material_key::Kind>,
}

mod scene_dat_material_key {
    use super::*;

    #[derive(Clone, PartialEq, Oneof)]
    pub enum Kind {
        #[prost(message, tag = "1")]
        BuildBlock(SceneDatBuildBlock),
        #[prost(message, tag = "2")]
        FenceStake(EmptyDat),
        #[prost(message, tag = "3")]
        FenceStick(EmptyDat),
        #[prost(message, tag = "4")]
        TreeTrunk(SceneDatTreeVariant),
        #[prost(message, tag = "5")]
        TreeMain(SceneDatTreeVariant),
        #[prost(message, tag = "6")]
        TreeCrown(SceneDatTreeVariant),
    }
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatBuildBlock {
    #[prost(uint32, tag = "1")]
    index: u32,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatTreeVariant {
    #[prost(uint32, tag = "1")]
    variant: u32,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatPrimitiveParams {
    #[prost(oneof = "scene_dat_primitive_params::Kind", tags = "1, 2, 3")]
    kind: Option<scene_dat_primitive_params::Kind>,
}

mod scene_dat_primitive_params {
    use super::*;

    #[derive(Clone, PartialEq, Oneof)]
    pub enum Kind {
        #[prost(message, tag = "1")]
        Capsule(SceneDatCapsuleParams),
        #[prost(message, tag = "2")]
        ConicalFrustum(SceneDatConicalFrustumParams),
        #[prost(message, tag = "3")]
        Torus(SceneDatTorusParams),
    }
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatCapsuleParams {
    #[prost(float, tag = "1")]
    radius: f32,
    #[prost(float, tag = "2")]
    half_length: f32,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatConicalFrustumParams {
    #[prost(float, tag = "1")]
    radius_top: f32,
    #[prost(float, tag = "2")]
    radius_bottom: f32,
    #[prost(float, tag = "3")]
    height: f32,
}

#[derive(Clone, PartialEq, Message)]
struct SceneDatTorusParams {
    #[prost(float, tag = "1")]
    minor_radius: f32,
    #[prost(float, tag = "2")]
    major_radius: f32,
}

fn scene_dat_path(config: &AppConfig, active: &crate::realm::ActiveRealmScene) -> PathBuf {
    if let Some(path) = config.scene_dat_path.as_ref().cloned() {
        return path;
    }

    crate::realm::scene_dat_path(active)
}

fn workspace_scene_dat_path(
    config: &AppConfig,
    active: &crate::realm::ActiveRealmScene,
    tab: WorkspaceTab,
) -> PathBuf {
    let base = scene_dat_path(config, active);
    match tab {
        WorkspaceTab::ObjectPreview => base,
        WorkspaceTab::SceneBuild => {
            let dir = base.parent().unwrap_or_else(|| Path::new("."));
            dir.join("scene.build.dat")
        }
    }
}

fn quantize_world(value_m: f32, units_per_meter: u32) -> i32 {
    let units_per_meter = units_per_meter.max(1) as f32;
    (value_m * units_per_meter).round() as i32
}

fn dequantize_world(value_units: i32, units_per_meter: u32) -> f32 {
    let units_per_meter = units_per_meter.max(1) as f32;
    (value_units as f32) / units_per_meter
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }

    let tmp_path = path.with_extension("tmp");
    std::fs::write(&tmp_path, bytes)?;

    if let Err(err) = std::fs::rename(&tmp_path, path) {
        if path.exists() {
            let _ = std::fs::remove_file(path);
            std::fs::rename(&tmp_path, path)?;
        } else {
            return Err(err);
        }
    }

    Ok(())
}

fn u128_to_uuid(value: u128) -> Uuid128Dat {
    Uuid128Dat {
        hi: (value >> 64) as u64,
        lo: value as u64,
    }
}

fn uuid_to_u128(value: &Uuid128Dat) -> u128 {
    ((value.hi as u128) << 64) | (value.lo as u128)
}

fn pack_color(color: Color) -> ColorDat {
    let [r, g, b, a] = to_rgba8(color);
    ColorDat {
        rgba: u32::from_le_bytes([r, g, b, a]),
    }
}

fn unpack_color(color: &ColorDat) -> Color {
    let [r, g, b, a] = color.rgba.to_le_bytes();
    Color::srgba(
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
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

fn pack_non_default_scale(value: f32) -> Option<Float32Dat> {
    if !value.is_finite() {
        return None;
    }
    ((value - 1.0).abs() > 1e-4).then_some(Float32Dat { value })
}

fn mesh_key_to_dat(mesh: MeshKey) -> i32 {
    match mesh {
        MeshKey::UnitCube => SceneDatMeshKey::UnitCube as i32,
        MeshKey::UnitCylinder => SceneDatMeshKey::UnitCylinder as i32,
        MeshKey::UnitCone => SceneDatMeshKey::UnitCone as i32,
        MeshKey::UnitSphere => SceneDatMeshKey::UnitSphere as i32,
        MeshKey::UnitPlane => SceneDatMeshKey::UnitPlane as i32,
        MeshKey::UnitCapsule => SceneDatMeshKey::UnitCapsule as i32,
        MeshKey::UnitConicalFrustum => SceneDatMeshKey::UnitConicalFrustum as i32,
        MeshKey::UnitTorus => SceneDatMeshKey::UnitTorus as i32,
        MeshKey::UnitTriangle => SceneDatMeshKey::UnitTriangle as i32,
        MeshKey::UnitTetrahedron => SceneDatMeshKey::UnitTetrahedron as i32,
        MeshKey::TreeTrunk => SceneDatMeshKey::TreeTrunk as i32,
        MeshKey::TreeCone => SceneDatMeshKey::TreeCone as i32,
    }
}

fn mesh_key_from_dat(mesh: i32) -> Result<MeshKey, String> {
    let mesh = SceneDatMeshKey::try_from(mesh).map_err(|_| format!("Unknown mesh key {}", mesh))?;
    Ok(match mesh {
        SceneDatMeshKey::UnitCube => MeshKey::UnitCube,
        SceneDatMeshKey::UnitCylinder => MeshKey::UnitCylinder,
        SceneDatMeshKey::UnitCone => MeshKey::UnitCone,
        SceneDatMeshKey::UnitSphere => MeshKey::UnitSphere,
        SceneDatMeshKey::UnitPlane => MeshKey::UnitPlane,
        SceneDatMeshKey::UnitCapsule => MeshKey::UnitCapsule,
        SceneDatMeshKey::UnitConicalFrustum => MeshKey::UnitConicalFrustum,
        SceneDatMeshKey::UnitTorus => MeshKey::UnitTorus,
        SceneDatMeshKey::UnitTriangle => MeshKey::UnitTriangle,
        SceneDatMeshKey::UnitTetrahedron => MeshKey::UnitTetrahedron,
        SceneDatMeshKey::TreeTrunk => MeshKey::TreeTrunk,
        SceneDatMeshKey::TreeCone => MeshKey::TreeCone,
    })
}

fn material_key_to_dat(key: MaterialKey) -> SceneDatMaterialKey {
    let kind = match key {
        MaterialKey::BuildBlock { index } => {
            scene_dat_material_key::Kind::BuildBlock(SceneDatBuildBlock {
                index: index as u32,
            })
        }
        MaterialKey::FenceStake => scene_dat_material_key::Kind::FenceStake(EmptyDat {}),
        MaterialKey::FenceStick => scene_dat_material_key::Kind::FenceStick(EmptyDat {}),
        MaterialKey::TreeTrunk { variant } => {
            scene_dat_material_key::Kind::TreeTrunk(SceneDatTreeVariant {
                variant: variant as u32,
            })
        }
        MaterialKey::TreeMain { variant } => {
            scene_dat_material_key::Kind::TreeMain(SceneDatTreeVariant {
                variant: variant as u32,
            })
        }
        MaterialKey::TreeCrown { variant } => {
            scene_dat_material_key::Kind::TreeCrown(SceneDatTreeVariant {
                variant: variant as u32,
            })
        }
    };

    SceneDatMaterialKey { kind: Some(kind) }
}

fn material_key_from_dat(key: &SceneDatMaterialKey) -> Result<MaterialKey, String> {
    let Some(kind) = key.kind.as_ref() else {
        return Err("Missing material kind".into());
    };
    Ok(match kind {
        scene_dat_material_key::Kind::BuildBlock(b) => MaterialKey::BuildBlock {
            index: b.index as usize,
        },
        scene_dat_material_key::Kind::FenceStake(_) => MaterialKey::FenceStake,
        scene_dat_material_key::Kind::FenceStick(_) => MaterialKey::FenceStick,
        scene_dat_material_key::Kind::TreeTrunk(v) => MaterialKey::TreeTrunk {
            variant: v.variant as usize,
        },
        scene_dat_material_key::Kind::TreeMain(v) => MaterialKey::TreeMain {
            variant: v.variant as usize,
        },
        scene_dat_material_key::Kind::TreeCrown(v) => MaterialKey::TreeCrown {
            variant: v.variant as usize,
        },
    })
}

fn params_to_dat(params: PrimitiveParams) -> SceneDatPrimitiveParams {
    let kind = match params {
        PrimitiveParams::Capsule {
            radius,
            half_length,
        } => scene_dat_primitive_params::Kind::Capsule(SceneDatCapsuleParams {
            radius,
            half_length,
        }),
        PrimitiveParams::ConicalFrustum {
            radius_top,
            radius_bottom,
            height,
        } => scene_dat_primitive_params::Kind::ConicalFrustum(SceneDatConicalFrustumParams {
            radius_top,
            radius_bottom,
            height,
        }),
        PrimitiveParams::Torus {
            minor_radius,
            major_radius,
        } => scene_dat_primitive_params::Kind::Torus(SceneDatTorusParams {
            minor_radius,
            major_radius,
        }),
    };

    SceneDatPrimitiveParams { kind: Some(kind) }
}

fn params_from_dat(params: &SceneDatPrimitiveParams) -> Result<PrimitiveParams, String> {
    let Some(kind) = params.kind.as_ref() else {
        return Err("Missing primitive params kind".into());
    };
    Ok(match kind {
        scene_dat_primitive_params::Kind::Capsule(p) => PrimitiveParams::Capsule {
            radius: p.radius,
            half_length: p.half_length,
        },
        scene_dat_primitive_params::Kind::ConicalFrustum(p) => PrimitiveParams::ConicalFrustum {
            radius_top: p.radius_top,
            radius_bottom: p.radius_bottom,
            height: p.height,
        },
        scene_dat_primitive_params::Kind::Torus(p) => PrimitiveParams::Torus {
            minor_radius: p.minor_radius,
            major_radius: p.major_radius,
        },
    })
}

fn collider_to_dat(collider: ColliderProfile) -> SceneDatCollider {
    let kind = match collider {
        ColliderProfile::None => scene_dat_collider::Kind::None(EmptyDat {}),
        ColliderProfile::CircleXZ { radius } => {
            scene_dat_collider::Kind::CircleXz(SceneDatCircleXz { radius })
        }
        ColliderProfile::AabbXZ { half_extents } => {
            scene_dat_collider::Kind::AabbXz(SceneDatAabbXz {
                half_x: half_extents.x,
                half_z: half_extents.y,
            })
        }
    };

    SceneDatCollider { kind: Some(kind) }
}

fn collider_from_dat(collider: &SceneDatCollider) -> Result<ColliderProfile, String> {
    let Some(kind) = collider.kind.as_ref() else {
        return Err("Missing collider kind".into());
    };
    Ok(match kind {
        scene_dat_collider::Kind::None(_) => ColliderProfile::None,
        scene_dat_collider::Kind::CircleXz(c) => ColliderProfile::CircleXZ { radius: c.radius },
        scene_dat_collider::Kind::AabbXz(a) => ColliderProfile::AabbXZ {
            half_extents: Vec2::new(a.half_x, a.half_z),
        },
    })
}

fn interaction_to_dat(interaction: ObjectInteraction) -> SceneDatInteraction {
    let movement_block = interaction.movement_block.map(|rule| match rule {
        MovementBlockRule::Always => SceneDatMovementBlock {
            kind: Some(scene_dat_movement_block::Kind::Always(EmptyDat {})),
        },
        MovementBlockRule::UpperBodyFraction(fraction) => SceneDatMovementBlock {
            kind: Some(scene_dat_movement_block::Kind::UpperBodyFraction(
                Float32Dat { value: fraction },
            )),
        },
    });

    SceneDatInteraction {
        blocks_bullets: interaction.blocks_bullets,
        blocks_laser: interaction.blocks_laser,
        movement_block,
        supports_standing: interaction.supports_standing,
    }
}

fn interaction_from_dat(interaction: &SceneDatInteraction) -> Result<ObjectInteraction, String> {
    let movement_block = match interaction.movement_block.as_ref() {
        None => None,
        Some(m) => {
            let Some(kind) = m.kind.as_ref() else {
                return Err("Missing movement_block kind".into());
            };
            Some(match kind {
                scene_dat_movement_block::Kind::Always(_) => MovementBlockRule::Always,
                scene_dat_movement_block::Kind::UpperBodyFraction(f) => {
                    MovementBlockRule::UpperBodyFraction(f.value)
                }
            })
        }
    };

    Ok(ObjectInteraction {
        blocks_bullets: interaction.blocks_bullets,
        blocks_laser: interaction.blocks_laser,
        movement_block,
        supports_standing: interaction.supports_standing,
    })
}

fn mobility_to_dat(mobility: MobilityDef) -> SceneDatMobility {
    let mode = match mobility.mode {
        MobilityMode::Ground => SceneDatMobilityMode::Ground as i32,
        MobilityMode::Air => SceneDatMobilityMode::Air as i32,
    };
    SceneDatMobility {
        mode,
        max_speed: mobility.max_speed,
    }
}

fn mobility_from_dat(mobility: &SceneDatMobility) -> Option<MobilityDef> {
    let mode = match mobility.mode {
        x if x == SceneDatMobilityMode::Ground as i32 => MobilityMode::Ground,
        x if x == SceneDatMobilityMode::Air as i32 => MobilityMode::Air,
        _ => return None,
    };
    let max_speed = mobility.max_speed;
    if !max_speed.is_finite() || max_speed <= 0.0 {
        return None;
    }
    Some(MobilityDef { mode, max_speed })
}

fn transform_to_dat(transform: &Transform) -> SceneDatTransform {
    SceneDatTransform {
        tx: transform.translation.x,
        ty: transform.translation.y,
        tz: transform.translation.z,
        rx: transform.rotation.x,
        ry: transform.rotation.y,
        rz: transform.rotation.z,
        rw: transform.rotation.w,
        sx: transform.scale.x,
        sy: transform.scale.y,
        sz: transform.scale.z,
    }
}

fn transform_from_dat(transform: &SceneDatTransform) -> Transform {
    Transform {
        translation: Vec3::new(transform.tx, transform.ty, transform.tz),
        rotation: Quat::from_xyzw(transform.rx, transform.ry, transform.rz, transform.rw),
        scale: Vec3::new(transform.sx, transform.sy, transform.sz),
    }
}

fn primitive_to_dat(primitive: &PrimitiveVisualDef) -> SceneDatPrimitive {
    let kind = match primitive {
        PrimitiveVisualDef::Mesh { mesh, material } => {
            scene_dat_primitive::Kind::MeshRef(SceneDatPrimitiveMeshRef {
                mesh: mesh_key_to_dat(*mesh),
                material: Some(material_key_to_dat(*material)),
            })
        }
        PrimitiveVisualDef::Primitive {
            mesh,
            params,
            color,
            unlit,
        } => scene_dat_primitive::Kind::Solid(SceneDatPrimitiveSolid {
            mesh: mesh_key_to_dat(*mesh),
            params: params.map(params_to_dat),
            color: Some(pack_color(*color)),
            unlit: *unlit,
        }),
    };

    SceneDatPrimitive { kind: Some(kind) }
}

fn primitive_from_dat(primitive: &SceneDatPrimitive) -> Result<PrimitiveVisualDef, String> {
    let Some(kind) = primitive.kind.as_ref() else {
        return Err("Missing primitive kind".into());
    };
    Ok(match kind {
        scene_dat_primitive::Kind::MeshRef(m) => {
            let mesh = mesh_key_from_dat(m.mesh)?;
            let Some(material) = m.material.as_ref() else {
                return Err("Missing primitive mesh material".into());
            };
            PrimitiveVisualDef::Mesh {
                mesh,
                material: material_key_from_dat(material)?,
            }
        }
        scene_dat_primitive::Kind::Solid(s) => {
            let mesh = mesh_key_from_dat(s.mesh)?;
            let params = match s.params.as_ref() {
                Some(p) => Some(params_from_dat(p)?),
                None => None,
            };
            let color = s
                .color
                .as_ref()
                .map(unpack_color)
                .unwrap_or(Color::srgba(0.75, 0.75, 0.78, 1.0));
            PrimitiveVisualDef::Primitive {
                mesh,
                params,
                color,
                unlit: s.unlit,
            }
        }
    })
}

fn part_animation_spec_to_dat(spec: &PartAnimationSpec) -> SceneDatPartAnimation {
    let kind = match &spec.clip {
        PartAnimationDef::Loop {
            duration_secs,
            keyframes,
        } => scene_dat_part_animation::Kind::Loop(SceneDatPartAnimationLoop {
            duration_secs: *duration_secs,
            keyframes: keyframes
                .iter()
                .map(|kf| SceneDatPartAnimationKeyframe {
                    time_secs: kf.time_secs,
                    delta: Some(transform_to_dat(&kf.delta)),
                })
                .collect(),
        }),
        PartAnimationDef::Once {
            duration_secs,
            keyframes,
        } => scene_dat_part_animation::Kind::Once(SceneDatPartAnimationLoop {
            duration_secs: *duration_secs,
            keyframes: keyframes
                .iter()
                .map(|kf| SceneDatPartAnimationKeyframe {
                    time_secs: kf.time_secs,
                    delta: Some(transform_to_dat(&kf.delta)),
                })
                .collect(),
        }),
        PartAnimationDef::PingPong {
            duration_secs,
            keyframes,
        } => scene_dat_part_animation::Kind::PingPong(SceneDatPartAnimationLoop {
            duration_secs: *duration_secs,
            keyframes: keyframes
                .iter()
                .map(|kf| SceneDatPartAnimationKeyframe {
                    time_secs: kf.time_secs,
                    delta: Some(transform_to_dat(&kf.delta)),
                })
                .collect(),
        }),
        PartAnimationDef::Spin {
            axis,
            radians_per_unit,
            axis_space,
        } => scene_dat_part_animation::Kind::Spin(SceneDatPartAnimationSpin {
            axis_x: axis.x,
            axis_y: axis.y,
            axis_z: axis.z,
            radians_per_unit: *radians_per_unit,
            axis_space: match axis_space {
                crate::object::registry::PartAnimationSpinAxisSpace::Join => {
                    SceneDatPartAnimationSpinAxisSpace::Join as i32
                }
                crate::object::registry::PartAnimationSpinAxisSpace::ChildLocal => {
                    SceneDatPartAnimationSpinAxisSpace::ChildLocal as i32
                }
            },
        }),
    };

    let driver = match spec.driver {
        PartAnimationDriver::Always => SceneDatPartAnimationDriver::Always as i32,
        PartAnimationDriver::MovePhase => SceneDatPartAnimationDriver::MovePhase as i32,
        PartAnimationDriver::MoveDistance => SceneDatPartAnimationDriver::MoveDistance as i32,
        PartAnimationDriver::AttackTime => SceneDatPartAnimationDriver::AttackTime as i32,
    };
    let speed_scale = if spec.speed_scale.is_finite() && spec.speed_scale > 0.0 {
        spec.speed_scale
    } else {
        1.0
    };
    let time_offset_units = if spec.time_offset_units.is_finite() {
        spec.time_offset_units
    } else {
        0.0
    };

    SceneDatPartAnimation {
        kind: Some(kind),
        driver,
        speed_scale,
        time_offset_units,
    }
}

fn part_animation_spec_from_dat(animation: &SceneDatPartAnimation) -> Option<PartAnimationSpec> {
    let kind = animation.kind.as_ref()?;
    let driver = match animation.driver {
        x if x == SceneDatPartAnimationDriver::MovePhase as i32 => PartAnimationDriver::MovePhase,
        x if x == SceneDatPartAnimationDriver::MoveDistance as i32 => {
            PartAnimationDriver::MoveDistance
        }
        x if x == SceneDatPartAnimationDriver::AttackTime as i32 => PartAnimationDriver::AttackTime,
        _ => PartAnimationDriver::Always,
    };
    let speed_scale = if animation.speed_scale.is_finite() && animation.speed_scale > 0.0 {
        animation.speed_scale
    } else {
        1.0
    };
    let time_offset_units = if animation.time_offset_units.is_finite() {
        animation.time_offset_units
    } else {
        0.0
    };

    let clip = match kind {
        scene_dat_part_animation::Kind::Loop(loop_)
        | scene_dat_part_animation::Kind::Once(loop_)
        | scene_dat_part_animation::Kind::PingPong(loop_) => {
            let duration_secs = loop_.duration_secs;
            if !duration_secs.is_finite() || duration_secs <= 0.0 {
                return None;
            }
            let mut keyframes: Vec<PartAnimationKeyframeDef> =
                Vec::with_capacity(loop_.keyframes.len());
            for kf in &loop_.keyframes {
                if !kf.time_secs.is_finite() {
                    continue;
                }
                let delta = kf
                    .delta
                    .as_ref()
                    .map(transform_from_dat)
                    .unwrap_or(Transform::IDENTITY);
                keyframes.push(PartAnimationKeyframeDef {
                    time_secs: kf.time_secs.clamp(0.0, duration_secs),
                    delta,
                });
            }
            if keyframes.is_empty() {
                return None;
            }
            keyframes.sort_by(|a, b| {
                a.time_secs
                    .partial_cmp(&b.time_secs)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            match kind {
                scene_dat_part_animation::Kind::Loop(_) => PartAnimationDef::Loop {
                    duration_secs,
                    keyframes,
                },
                scene_dat_part_animation::Kind::Once(_) => PartAnimationDef::Once {
                    duration_secs,
                    keyframes,
                },
                scene_dat_part_animation::Kind::PingPong(_) => PartAnimationDef::PingPong {
                    duration_secs,
                    keyframes,
                },
                scene_dat_part_animation::Kind::Spin(_) => unreachable!("spin handled below"),
            }
        }
        scene_dat_part_animation::Kind::Spin(spin) => {
            let axis = Vec3::new(spin.axis_x, spin.axis_y, spin.axis_z);
            let radians_per_unit = spin.radians_per_unit;
            if !axis.is_finite() || !radians_per_unit.is_finite() || radians_per_unit.abs() <= 1e-6
            {
                return None;
            }
            let axis_space =
                match SceneDatPartAnimationSpinAxisSpace::try_from(spin.axis_space).ok() {
                    Some(SceneDatPartAnimationSpinAxisSpace::ChildLocal) => {
                        crate::object::registry::PartAnimationSpinAxisSpace::ChildLocal
                    }
                    _ => crate::object::registry::PartAnimationSpinAxisSpace::Join,
                };
            PartAnimationDef::Spin {
                axis,
                radians_per_unit,
                axis_space,
            }
        }
    };

    Some(PartAnimationSpec {
        driver,
        speed_scale,
        time_offset_units,
        clip,
    })
}

fn part_animation_slot_to_dat(slot: &PartAnimationSlot) -> SceneDatPartAnimationSlot {
    SceneDatPartAnimationSlot {
        channel: slot.channel.to_string(),
        animation: Some(part_animation_spec_to_dat(&slot.spec)),
    }
}

fn part_animation_slot_from_dat(slot: &SceneDatPartAnimationSlot) -> Option<PartAnimationSlot> {
    let channel = slot.channel.trim();
    if channel.is_empty() {
        return None;
    }
    let spec = slot
        .animation
        .as_ref()
        .and_then(part_animation_spec_from_dat)?;
    Some(PartAnimationSlot {
        channel: channel.to_string().into(),
        spec,
    })
}

fn part_to_dat(part: &ObjectPartDef) -> SceneDatPartDef {
    let kind = match &part.kind {
        ObjectPartKind::ObjectRef { object_id } => {
            scene_dat_part_def::Kind::ObjectRef(u128_to_uuid(*object_id))
        }
        ObjectPartKind::Primitive { primitive } => {
            scene_dat_part_def::Kind::Primitive(primitive_to_dat(primitive))
        }
        ObjectPartKind::Model { scene } => scene_dat_part_def::Kind::Model(scene.to_string()),
    };

    SceneDatPartDef {
        part_id: part.part_id.map(u128_to_uuid),
        transform: Some(transform_to_dat(&part.transform)),
        kind: Some(kind),
        attachment: part.attachment.as_ref().map(|a| SceneDatAttachment {
            parent_anchor: a.parent_anchor.to_string(),
            child_anchor: a.child_anchor.to_string(),
        }),
        animations: part
            .animations
            .iter()
            .map(part_animation_slot_to_dat)
            .collect(),
    }
}

fn part_from_dat(part: &SceneDatPartDef) -> Result<ObjectPartDef, String> {
    let Some(kind) = part.kind.as_ref() else {
        return Err("Missing part kind".into());
    };
    let transform = part
        .transform
        .as_ref()
        .map(transform_from_dat)
        .unwrap_or(Transform::IDENTITY);

    let kind = match kind {
        scene_dat_part_def::Kind::ObjectRef(id) => ObjectPartKind::ObjectRef {
            object_id: uuid_to_u128(id),
        },
        scene_dat_part_def::Kind::Primitive(p) => ObjectPartKind::Primitive {
            primitive: primitive_from_dat(p)?,
        },
        scene_dat_part_def::Kind::Model(scene) => ObjectPartKind::Model {
            scene: scene.clone().into(),
        },
    };

    Ok(ObjectPartDef {
        part_id: part.part_id.as_ref().map(uuid_to_u128),
        render_priority: None,
        kind,
        attachment: part.attachment.as_ref().and_then(|a| {
            let parent_anchor = a.parent_anchor.trim();
            let child_anchor = a.child_anchor.trim();
            if parent_anchor.is_empty() || child_anchor.is_empty() {
                return None;
            }
            Some(AttachmentDef {
                parent_anchor: parent_anchor.to_string().into(),
                child_anchor: child_anchor.to_string().into(),
            })
        }),
        animations: part
            .animations
            .iter()
            .filter_map(part_animation_slot_from_dat)
            .collect(),
        transform,
    })
}

fn def_to_dat(def: &ObjectDef) -> SceneDatObjectDef {
    SceneDatObjectDef {
        object_id: Some(u128_to_uuid(def.object_id)),
        label: def.label.to_string(),
        size_x: def.size.x,
        size_y: def.size.y,
        size_z: def.size.z,
        ground_origin_y: def
            .ground_origin_y
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| Float32Dat { value }),
        collider: Some(collider_to_dat(def.collider)),
        interaction: Some(interaction_to_dat(def.interaction)),
        anchors: def
            .anchors
            .iter()
            .map(|a| SceneDatAnchorDef {
                name: a.name.to_string(),
                transform: Some(transform_to_dat(&a.transform)),
            })
            .collect(),
        parts: def.parts.iter().map(part_to_dat).collect(),
        minimap_color: def.minimap_color.map(pack_color),
        health_bar_offset_y: def.health_bar_offset_y.map(|value| Float32Dat { value }),
        mobility: def.mobility.map(mobility_to_dat),
        projectile: def.projectile.map(projectile_to_dat),
        attack: def.attack.as_ref().map(attack_to_dat),
        aim: def.aim.as_ref().map(|aim| SceneDatAimProfile {
            max_yaw_delta_degrees: aim.max_yaw_delta_degrees.map(|value| Float32Dat { value }),
            components: aim.components.iter().copied().map(u128_to_uuid).collect(),
        }),
    }
}

fn def_from_dat(def: &SceneDatObjectDef) -> Result<ObjectDef, String> {
    let Some(id) = def.object_id.as_ref() else {
        return Err("Missing object id".into());
    };
    let object_id = uuid_to_u128(id);

    let ground_origin_y = def
        .ground_origin_y
        .as_ref()
        .map(|value| value.value)
        .filter(|value| value.is_finite() && *value >= 0.0);

    let collider = match def.collider.as_ref() {
        Some(c) => collider_from_dat(c)?,
        None => ColliderProfile::None,
    };
    let interaction = match def.interaction.as_ref() {
        Some(i) => interaction_from_dat(i)?,
        None => ObjectInteraction::none(),
    };

    let mut parts = Vec::with_capacity(def.parts.len());
    for part in &def.parts {
        parts.push(part_from_dat(part)?);
    }

    let mut anchors = Vec::with_capacity(def.anchors.len());
    for anchor in &def.anchors {
        let name = anchor.name.trim();
        if name.is_empty() {
            continue;
        }
        let transform = anchor
            .transform
            .as_ref()
            .map(transform_from_dat)
            .unwrap_or(Transform::IDENTITY);
        anchors.push(AnchorDef {
            name: name.to_string().into(),
            transform,
        });
    }

    let aim = def
        .aim
        .as_ref()
        .map(|aim| crate::object::registry::AimProfile {
            max_yaw_delta_degrees: aim.max_yaw_delta_degrees.as_ref().map(|v| v.value),
            components: aim.components.iter().map(uuid_to_u128).collect::<Vec<_>>(),
        });

    Ok(ObjectDef {
        object_id,
        label: def.label.clone().into(),
        size: Vec3::new(def.size_x, def.size_y, def.size_z),
        ground_origin_y,
        collider,
        interaction,
        aim,
        mobility: def.mobility.as_ref().and_then(mobility_from_dat),
        anchors,
        parts,
        minimap_color: def.minimap_color.as_ref().map(unpack_color),
        health_bar_offset_y: def.health_bar_offset_y.as_ref().map(|v| v.value),
        enemy: None,
        muzzle: None,
        projectile: def
            .projectile
            .as_ref()
            .map(projectile_from_dat)
            .transpose()?,
        attack: def.attack.as_ref().map(attack_from_dat).transpose()?,
    })
}

fn gather_referenced_defs(
    library: &ObjectLibrary,
    roots: impl IntoIterator<Item = u128>,
) -> Vec<u128> {
    let mut visited: HashSet<u128> = HashSet::new();
    let mut queue: VecDeque<u128> = roots.into_iter().collect();
    while let Some(object_id) = queue.pop_front() {
        if !visited.insert(object_id) {
            continue;
        }
        let Some(def) = library.get(object_id) else {
            warn!("scene.dat: missing object def for {object_id:#x}");
            continue;
        };
        if let Some(attack) = def.attack.as_ref() {
            if let Some(ranged) = attack.ranged.as_ref() {
                if !visited.contains(&ranged.projectile_prefab) {
                    queue.push_back(ranged.projectile_prefab);
                }
                if !visited.contains(&ranged.muzzle.object_id) {
                    queue.push_back(ranged.muzzle.object_id);
                }
            }
        }
        for part in &def.parts {
            if let ObjectPartKind::ObjectRef { object_id: child } = &part.kind {
                if !visited.contains(child) {
                    queue.push_back(*child);
                }
            }
        }
    }

    let mut ids: Vec<u128> = visited.into_iter().collect();
    ids.sort_unstable();
    ids
}

fn projectile_to_dat(projectile: ProjectileProfile) -> SceneDatProjectile {
    SceneDatProjectile {
        obstacle_rule: match projectile.obstacle_rule {
            ProjectileObstacleRule::BulletsBlockers => {
                SceneDatProjectileObstacleRule::BulletsBlockers as i32
            }
            ProjectileObstacleRule::LaserBlockers => {
                SceneDatProjectileObstacleRule::LaserBlockers as i32
            }
        },
        speed: projectile.speed,
        ttl_secs: projectile.ttl_secs,
        damage: projectile.damage,
        spawn_energy_impact: projectile.spawn_energy_impact,
    }
}

fn projectile_from_dat(projectile: &SceneDatProjectile) -> Result<ProjectileProfile, String> {
    let obstacle_rule = match SceneDatProjectileObstacleRule::try_from(projectile.obstacle_rule)
        .unwrap_or(SceneDatProjectileObstacleRule::BulletsBlockers)
    {
        SceneDatProjectileObstacleRule::BulletsBlockers => ProjectileObstacleRule::BulletsBlockers,
        SceneDatProjectileObstacleRule::LaserBlockers => ProjectileObstacleRule::LaserBlockers,
    };
    Ok(ProjectileProfile {
        obstacle_rule,
        speed: projectile.speed,
        ttl_secs: projectile.ttl_secs,
        damage: projectile.damage,
        spawn_energy_impact: projectile.spawn_energy_impact,
    })
}

fn attack_to_dat(attack: &UnitAttackProfile) -> SceneDatUnitAttack {
    SceneDatUnitAttack {
        kind: match attack.kind {
            UnitAttackKind::Melee => SceneDatUnitAttackKind::Melee as i32,
            UnitAttackKind::RangedProjectile => SceneDatUnitAttackKind::RangedProjectile as i32,
        },
        cooldown_secs: attack.cooldown_secs,
        damage: attack.damage,
        anim_window_secs: attack.anim_window_secs,
        melee: attack.melee.map(|melee| SceneDatMeleeAttack {
            range: melee.range,
            radius: melee.radius,
            arc_degrees: melee.arc_degrees,
        }),
        ranged: attack.ranged.as_ref().map(|ranged| SceneDatRangedAttack {
            projectile_prefab: Some(u128_to_uuid(ranged.projectile_prefab)),
            muzzle: Some(SceneDatAnchorRef {
                object_id: Some(u128_to_uuid(ranged.muzzle.object_id)),
                anchor: ranged.muzzle.anchor.to_string(),
            }),
        }),
    }
}

fn attack_from_dat(attack: &SceneDatUnitAttack) -> Result<UnitAttackProfile, String> {
    let kind =
        SceneDatUnitAttackKind::try_from(attack.kind).unwrap_or(SceneDatUnitAttackKind::Unknown);
    match kind {
        SceneDatUnitAttackKind::Unknown => Err("Unknown attack kind".into()),
        SceneDatUnitAttackKind::Melee => {
            let melee = attack
                .melee
                .as_ref()
                .ok_or_else(|| "Melee attack missing melee profile".to_string())?;
            Ok(UnitAttackProfile {
                kind: UnitAttackKind::Melee,
                cooldown_secs: attack.cooldown_secs,
                damage: attack.damage,
                anim_window_secs: attack.anim_window_secs,
                melee: Some(MeleeAttackProfile {
                    range: melee.range,
                    radius: melee.radius,
                    arc_degrees: melee.arc_degrees,
                }),
                ranged: None,
            })
        }
        SceneDatUnitAttackKind::RangedProjectile => {
            let ranged = attack
                .ranged
                .as_ref()
                .ok_or_else(|| "Ranged attack missing ranged profile".to_string())?;
            let projectile_prefab = ranged
                .projectile_prefab
                .as_ref()
                .map(uuid_to_u128)
                .ok_or_else(|| "Ranged attack missing projectile prefab".to_string())?;
            let muzzle = ranged
                .muzzle
                .as_ref()
                .ok_or_else(|| "Ranged attack missing muzzle".to_string())?;
            let muzzle_object_id = muzzle
                .object_id
                .as_ref()
                .map(uuid_to_u128)
                .ok_or_else(|| "Ranged attack missing muzzle object id".to_string())?;
            let anchor_name = muzzle.anchor.trim();
            if anchor_name.is_empty() {
                return Err("Ranged attack has empty muzzle anchor name".into());
            }
            Ok(UnitAttackProfile {
                kind: UnitAttackKind::RangedProjectile,
                cooldown_secs: attack.cooldown_secs,
                damage: attack.damage,
                anim_window_secs: attack.anim_window_secs,
                melee: None,
                ranged: Some(RangedAttackProfile {
                    projectile_prefab,
                    muzzle: AnchorRef {
                        object_id: muzzle_object_id,
                        anchor: anchor_name.to_string().into(),
                    },
                }),
            })
        }
    }
}

fn save_scene_dat_internal(
    objects: &Query<
        (
            &Transform,
            &ObjectId,
            &ObjectPrefabId,
            Option<&ObjectTint>,
            Option<&ObjectForms>,
            Option<&Player>,
        ),
        Or<(With<BuildObject>, With<Commandable>)>,
    >,
    library: &ObjectLibrary,
    path: &Path,
) -> io::Result<usize> {
    let units_per_meter = DEFAULT_UNITS_PER_METER;

    let mut instances: Vec<SceneDatObjectInstance> = Vec::with_capacity(objects.iter().len());
    let mut root_defs: Vec<u128> = Vec::with_capacity(objects.iter().len());

    let mut found_protagonist = false;

    for (transform, instance_id, prefab_id, tint, forms, player) in objects {
        let pos = transform.translation;
        let scale = transform.scale;

        let (forms, active) = if let Some(forms) = forms {
            let mut forms_list = forms.forms.clone();
            if forms_list.is_empty() {
                forms_list.push(prefab_id.0);
            }
            let mut active = forms.active.min(forms_list.len().saturating_sub(1));
            if forms_list.get(active).copied().unwrap_or(prefab_id.0) != prefab_id.0 {
                if let Some(found) = forms_list.iter().position(|v| *v == prefab_id.0) {
                    active = found;
                } else {
                    forms_list.clear();
                    forms_list.push(prefab_id.0);
                    active = 0;
                }
            }
            (forms_list, active)
        } else {
            (vec![prefab_id.0], 0usize)
        };

        root_defs.extend(forms.iter().copied());

        let is_protagonist = if player.is_some() {
            if found_protagonist {
                warn!("scene.dat: multiple Player Character entities found; only the first will be saved");
                false
            } else {
                found_protagonist = true;
                true
            }
        } else {
            false
        };

        instances.push(SceneDatObjectInstance {
            instance_id: Some(u128_to_uuid(instance_id.0)),
            base_object_id: Some(u128_to_uuid(prefab_id.0)),
            x_units: quantize_world(pos.x, units_per_meter),
            y_units: quantize_world(pos.y, units_per_meter),
            z_units: quantize_world(pos.z, units_per_meter),
            rot_x: transform.rotation.x,
            rot_y: transform.rotation.y,
            rot_z: transform.rotation.z,
            rot_w: transform.rotation.w,
            tint: tint.map(|t| pack_color(t.0)),
            scale_x: pack_non_default_scale(scale.x),
            scale_y: pack_non_default_scale(scale.y),
            scale_z: pack_non_default_scale(scale.z),
            forms: forms.into_iter().map(u128_to_uuid).collect(),
            active_form: active as u32,
            is_protagonist,
        });
    }

    instances.sort_by(|a, b| {
        a.base_object_id
            .as_ref()
            .map(|id| (id.hi, id.lo))
            .cmp(&b.base_object_id.as_ref().map(|id| (id.hi, id.lo)))
            .then_with(|| a.y_units.cmp(&b.y_units))
            .then_with(|| a.x_units.cmp(&b.x_units))
            .then_with(|| a.z_units.cmp(&b.z_units))
            .then_with(|| {
                a.instance_id
                    .as_ref()
                    .map(|id| (id.hi, id.lo))
                    .cmp(&b.instance_id.as_ref().map(|id| (id.hi, id.lo)))
            })
    });

    let def_ids = gather_referenced_defs(library, root_defs);
    let mut defs: Vec<SceneDatObjectDef> = Vec::with_capacity(def_ids.len());
    for object_id in def_ids {
        let Some(def) = library.get(object_id) else {
            continue;
        };
        defs.push(def_to_dat(def));
    }

    let scene = SceneDat {
        version: SCENE_DAT_VERSION,
        units_per_meter,
        defs,
        instances,
    };
    let bytes = scene.encode_to_vec();
    let instance_count = scene.instances.len();

    write_atomic(path, &bytes)?;

    Ok(instance_count)
}

fn spawn_build_object_from_scene(
    commands: &mut Commands,
    asset_server: &AssetServer,
    assets: &SceneAssets,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    material_cache: &mut visuals::MaterialCache,
    mesh_cache: &mut visuals::PrimitiveMeshCache,
    library: &ObjectLibrary,
    prefab_id: u128,
    mut transform: Transform,
    instance_id: ObjectId,
    tint: Option<Color>,
) -> Entity {
    let base_size = library
        .size(prefab_id)
        .unwrap_or_else(|| Vec3::splat(DEFAULT_OBJECT_SIZE_M));

    let mut scale = transform.scale;
    if !scale.x.is_finite() || scale.x.abs() < 1e-4 {
        scale.x = 1.0;
    }
    if !scale.y.is_finite() || scale.y.abs() < 1e-4 {
        scale.y = 1.0;
    }
    if !scale.z.is_finite() || scale.z.abs() < 1e-4 {
        scale.z = 1.0;
    }
    transform.scale = scale;

    let (yaw, _pitch, _roll) = transform.rotation.to_euler(EulerRot::YXZ);
    let c = yaw.cos().abs();
    let s = yaw.sin().abs();

    let (collider_half_xz, size) = match library.collider(prefab_id) {
        Some(ColliderProfile::CircleXZ { radius }) => {
            let r = radius.max(0.01) * scale.x.abs().max(scale.z.abs()).max(0.01);
            (
                Vec2::splat(r),
                Vec3::new(r * 2.0, base_size.y * scale.y.abs(), r * 2.0),
            )
        }
        Some(ColliderProfile::AabbXZ { half_extents }) => {
            let half = Vec2::new(
                half_extents.x.abs().max(0.01) * scale.x.abs().max(0.01),
                half_extents.y.abs().max(0.01) * scale.z.abs().max(0.01),
            );
            let rotated = Vec2::new(c * half.x + s * half.y, s * half.x + c * half.y);
            (
                rotated,
                Vec3::new(
                    rotated.x * 2.0,
                    base_size.y * scale.y.abs(),
                    rotated.y * 2.0,
                ),
            )
        }
        _ => {
            let half = Vec2::new(
                (base_size.x * 0.5).abs().max(0.01) * scale.x.abs().max(0.01),
                (base_size.z * 0.5).abs().max(0.01) * scale.z.abs().max(0.01),
            );
            let rotated = Vec2::new(c * half.x + s * half.y, s * half.x + c * half.y);
            (
                rotated,
                Vec3::new(
                    rotated.x * 2.0,
                    base_size.y * scale.y.abs(),
                    rotated.y * 2.0,
                ),
            )
        }
    };

    let mut entity_commands = commands.spawn((
        instance_id,
        ObjectPrefabId(prefab_id),
        BuildObject,
        BuildDimensions { size },
        AabbCollider {
            half_extents: collider_half_xz,
        },
        transform,
        Visibility::Inherited,
    ));
    if let Some(tint) = tint {
        entity_commands.insert(ObjectTint(tint));
    }
    visuals::spawn_object_visuals(
        &mut entity_commands,
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
    entity_commands.id()
}

fn spawn_unit_from_scene(
    commands: &mut Commands,
    asset_server: &AssetServer,
    assets: &SceneAssets,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    material_cache: &mut visuals::MaterialCache,
    mesh_cache: &mut visuals::PrimitiveMeshCache,
    library: &ObjectLibrary,
    prefab_id: u128,
    mut transform: Transform,
    instance_id: ObjectId,
    tint: Option<Color>,
) -> Entity {
    let base_size = library
        .size(prefab_id)
        .unwrap_or_else(|| Vec3::splat(DEFAULT_OBJECT_SIZE_M));

    let mut scale = transform.scale;
    if !scale.x.is_finite() || scale.x.abs() < 1e-4 {
        scale.x = 1.0;
    }
    if !scale.y.is_finite() || scale.y.abs() < 1e-4 {
        scale.y = 1.0;
    }
    if !scale.z.is_finite() || scale.z.abs() < 1e-4 {
        scale.z = 1.0;
    }
    transform.scale = scale;

    let radius = match library.collider(prefab_id) {
        Some(ColliderProfile::CircleXZ { radius }) => {
            radius.max(0.01) * scale.x.abs().max(scale.z.abs()).max(0.01)
        }
        Some(ColliderProfile::AabbXZ { half_extents }) => {
            let half = Vec2::new(
                half_extents.x.abs().max(0.01) * scale.x.abs().max(0.01),
                half_extents.y.abs().max(0.01) * scale.z.abs().max(0.01),
            );
            half.x.max(half.y)
        }
        _ => {
            let size = Vec2::new(
                (base_size.x * scale.x.abs()).abs().max(0.01),
                (base_size.z * scale.z.abs()).abs().max(0.01),
            );
            (size.x.max(size.y) * 0.5).max(0.01)
        }
    };

    let mut entity_commands = commands.spawn((
        instance_id,
        ObjectPrefabId(prefab_id),
        Commandable,
        Collider { radius },
        transform,
        Visibility::Inherited,
    ));
    if let Some(tint) = tint {
        entity_commands.insert(ObjectTint(tint));
    }

    visuals::spawn_object_visuals(
        &mut entity_commands,
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
    entity_commands.id()
}

fn spawn_scene_instance_from_scene(
    commands: &mut Commands,
    asset_server: &AssetServer,
    assets: &SceneAssets,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    material_cache: &mut visuals::MaterialCache,
    mesh_cache: &mut visuals::PrimitiveMeshCache,
    library: &ObjectLibrary,
    prefab_id: u128,
    transform: Transform,
    instance_id: ObjectId,
    tint: Option<Color>,
) -> Entity {
    if library.mobility(prefab_id).is_some() {
        spawn_unit_from_scene(
            commands,
            asset_server,
            assets,
            meshes,
            materials,
            material_cache,
            mesh_cache,
            library,
            prefab_id,
            transform,
            instance_id,
            tint,
        )
    } else {
        spawn_build_object_from_scene(
            commands,
            asset_server,
            assets,
            meshes,
            materials,
            material_cache,
            mesh_cache,
            library,
            prefab_id,
            transform,
            instance_id,
            tint,
        )
    }
}

pub(crate) fn load_scene_dat(
    mut commands: Commands,
    config: Res<AppConfig>,
    active: Res<crate::realm::ActiveRealmScene>,
    workspace_ui: Res<crate::workspace_ui::WorkspaceUiState>,
    asset_server: Res<AssetServer>,
    assets: Res<SceneAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut material_cache: ResMut<crate::object::visuals::MaterialCache>,
    mut mesh_cache: ResMut<crate::object::visuals::PrimitiveMeshCache>,
    mut library: ResMut<ObjectLibrary>,
) {
    // Reset library to builtins only. `scene.dat` must contain all defs needed to spawn.
    *library = ObjectLibrary::default();

    // Prefab descriptors are realm-level and loaded by UI/tooling on demand. Scene loading does not
    // depend on them, but we keep the cache so panels (Meta/Gen3D) stay stable across scene loads.

    let path = workspace_scene_dat_path(&config, &active, workspace_ui.tab);
    match load_scene_dat_from_path(
        &mut commands,
        &asset_server,
        &assets,
        &mut meshes,
        &mut materials,
        &mut material_cache,
        &mut mesh_cache,
        &mut library,
        &path,
    ) {
        Ok(spawned) => {
            if spawned > 0 {
                info!("Loaded {spawned} scene instances from {}.", path.display());
            }
        }
        Err(err) => {
            error!("{err}");
        }
    }
}

pub(crate) fn apply_pending_workspace_switch(
    mut commands: Commands,
    mut pending_switch: ResMut<PendingWorkspaceSwitch>,
    mut autosave: ResMut<SceneAutosaveState>,
    mut selection: ResMut<SelectionState>,
    mut world_drag: ResMut<crate::world_drag::WorldDragState>,
    mut deps: WorkspaceSwitchDeps,
    objects: Query<
        (
            &Transform,
            &ObjectId,
            &ObjectPrefabId,
            Option<&ObjectTint>,
            Option<&ObjectForms>,
            Option<&Player>,
        ),
        Or<(With<BuildObject>, With<Commandable>)>,
    >,
    existing_scene_entities: Query<Entity, Or<(With<BuildObject>, With<Commandable>)>>,
) {
    let Some(switch) = pending_switch.take() else {
        return;
    };
    if switch.from == switch.to {
        return;
    }

    info!(
        "Switching workspace: {} -> {}",
        switch.from.label(),
        switch.to.label()
    );

    let camera_snapshot = snapshot_camera(
        &deps.camera_zoom,
        &deps.camera_yaw,
        &deps.camera_pitch,
        &deps.camera_focus,
    );
    deps.workspace_camera.set(switch.from, camera_snapshot);
    let target_snapshot = deps.workspace_camera.get(switch.to);
    restore_camera(
        target_snapshot,
        &mut deps.camera_zoom,
        &mut deps.camera_yaw,
        &mut deps.camera_pitch,
        &mut deps.camera_focus,
    );

    let from_path = workspace_scene_dat_path(&deps.config, &deps.active, switch.from);
    match save_scene_dat_internal(&objects, &deps.library, &from_path) {
        Ok(instance_count) => {
            info!(
                "Saved {instance_count} scene instances to {} (workspace switch).",
                from_path.display()
            );
        }
        Err(err) => {
            warn!(
                "Failed to save {} before workspace switch: {err}",
                from_path.display()
            );
        }
    }

    selection.clear();
    *world_drag = crate::world_drag::WorldDragState::default();

    for entity in &existing_scene_entities {
        commands.entity(entity).try_despawn();
    }

    // Reset library to builtins only. `scene.dat` must contain all defs needed to spawn.
    *deps.library = ObjectLibrary::default();
    // Prefab descriptors are realm-level and reused across workspace tabs.

    let to_path = workspace_scene_dat_path(&deps.config, &deps.active, switch.to);
    match load_scene_dat_from_path(
        &mut commands,
        &deps.asset_server,
        &deps.assets,
        &mut deps.meshes,
        &mut deps.materials,
        &mut deps.material_cache,
        &mut deps.mesh_cache,
        &mut deps.library,
        &to_path,
    ) {
        Ok(spawned) => {
            if spawned > 0 {
                info!(
                    "Loaded {spawned} scene instances from {}.",
                    to_path.display()
                );
            }
        }
        Err(err) => {
            warn!("{err}");
        }
    }

    autosave.dirty = false;
    autosave.primed = false;
    autosave.timer.reset();
}

pub(crate) fn apply_pending_realm_scene_switch(
    mut commands: Commands,
    config: Res<AppConfig>,
    mut active: ResMut<crate::realm::ActiveRealmScene>,
    mut pending: ResMut<crate::realm::PendingRealmSceneSwitch>,
    mut autosave: ResMut<SceneAutosaveState>,
    mut workspace: ResMut<crate::scene_sources_runtime::SceneSourcesWorkspace>,
    workspace_ui: Res<crate::workspace_ui::WorkspaceUiState>,
    asset_server: Res<AssetServer>,
    assets: Res<SceneAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut material_cache: ResMut<crate::object::visuals::MaterialCache>,
    mut mesh_cache: ResMut<crate::object::visuals::PrimitiveMeshCache>,
    mut library: ResMut<ObjectLibrary>,
    mut prefab_descriptors: ResMut<crate::prefab_descriptors::PrefabDescriptorLibrary>,
    existing_scene_entities: Query<Entity, Or<(With<BuildObject>, With<Commandable>)>>,
) {
    let Some(target) = pending.target.take() else {
        return;
    };
    if target == *active {
        return;
    }

    let realm_changed = target.realm_id != active.realm_id;

    if let Err(err) = crate::realm::ensure_realm_scene_scaffold(&target.realm_id, &target.scene_id)
    {
        warn!("Scene switch aborted: {err}");
        return;
    }
    if let Err(err) = crate::realm::persist_active_selection(&target.realm_id, &target.scene_id) {
        warn!("Scene switch aborted: {err}");
        return;
    }

    info!(
        "Switching realm/scene: {}/{} -> {}/{}",
        active.realm_id, active.scene_id, target.realm_id, target.scene_id
    );
    *active = target;

    autosave.dirty = false;
    autosave.primed = false;
    autosave.timer.reset();

    workspace.loaded_from_dir = Some(crate::realm::scene_src_dir(&active));
    workspace.sources = None;

    for entity in &existing_scene_entities {
        commands.entity(entity).try_despawn();
    }

    // Reset library to builtins only. `scene.dat` must contain all defs needed to spawn.
    *library = ObjectLibrary::default();
    // Prefab descriptors are realm-level; keep them across scene switches so Meta/Gen3D UI stays
    // stable. Clear on realm switch to avoid stale data for a different realm.
    if realm_changed {
        prefab_descriptors.clear();
    }

    let path = workspace_scene_dat_path(&config, &active, workspace_ui.tab);
    match load_scene_dat_from_path(
        &mut commands,
        &asset_server,
        &assets,
        &mut meshes,
        &mut materials,
        &mut material_cache,
        &mut mesh_cache,
        &mut library,
        &path,
    ) {
        Ok(spawned) => {
            if spawned > 0 {
                info!("Loaded {spawned} scene instances from {}.", path.display());
            }
        }
        Err(err) => {
            warn!("{err}");
        }
    }
}

fn load_scene_dat_from_path(
    commands: &mut Commands,
    asset_server: &AssetServer,
    assets: &SceneAssets,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    material_cache: &mut crate::object::visuals::MaterialCache,
    mesh_cache: &mut crate::object::visuals::PrimitiveMeshCache,
    library: &mut ObjectLibrary,
    path: &Path,
) -> Result<usize, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(format!("Failed to read {}: {err}", path.display())),
    };

    let scene = SceneDat::decode(bytes.as_slice())
        .map_err(|err| format!("Failed to decode {}: {err}", path.display()))?;

    if !matches!(scene.version, 7 | 8 | SCENE_DAT_VERSION) {
        warn!(
            "Ignoring {}: unsupported scene.dat version {} (expected 7, 8, or {}).",
            path.display(),
            scene.version,
            SCENE_DAT_VERSION
        );
        return Ok(0);
    }

    for def in &scene.defs {
        match def_from_dat(def) {
            Ok(def) => library.upsert(def),
            Err(err) => {
                warn!("scene.dat: skipping invalid object def: {err}");
            }
        }
    }

    let units_per_meter = scene.units_per_meter.max(1);
    let mut spawned = 0usize;
    let hero_prefab_id = crate::object::types::characters::hero::object_id();
    let mut protagonist_entity: Option<(Entity, Transform)> = None;
    let mut hero_entity: Option<(Entity, Transform)> = None;

    for instance in &scene.instances {
        let Some(base_prefab_id) = instance.base_object_id.as_ref().map(uuid_to_u128) else {
            continue;
        };
        let mut forms: Vec<u128> = instance.forms.iter().map(uuid_to_u128).collect();
        if forms.is_empty() {
            forms.push(base_prefab_id);
        }
        let mut active = instance.active_form as usize;
        if active >= forms.len() {
            active = 0;
        }
        let prefab_id = forms.get(active).copied().unwrap_or(base_prefab_id);
        if library.get(prefab_id).is_none() {
            warn!("scene.dat: missing prefab for instance {prefab_id:#x}");
            continue;
        }

        let is_commandable_prefab = library.mobility(prefab_id).is_some();

        let instance_id = instance
            .instance_id
            .as_ref()
            .map(uuid_to_u128)
            .map(ObjectId)
            .unwrap_or_else(ObjectId::new_v4);

        let pos = Vec3::new(
            dequantize_world(instance.x_units, units_per_meter),
            dequantize_world(instance.y_units, units_per_meter),
            dequantize_world(instance.z_units, units_per_meter),
        );
        let rotation = Quat::from_xyzw(
            instance.rot_x,
            instance.rot_y,
            instance.rot_z,
            instance.rot_w,
        );
        let scale = Vec3::new(
            instance.scale_x.as_ref().map(|v| v.value).unwrap_or(1.0),
            instance.scale_y.as_ref().map(|v| v.value).unwrap_or(1.0),
            instance.scale_z.as_ref().map(|v| v.value).unwrap_or(1.0),
        );
        let transform = Transform::from_translation(pos)
            .with_rotation(rotation)
            .with_scale(scale);

        let tint = instance.tint.as_ref().map(unpack_color);

        let entity = spawn_scene_instance_from_scene(
            commands,
            asset_server,
            assets,
            meshes,
            materials,
            material_cache,
            mesh_cache,
            library,
            prefab_id,
            transform,
            instance_id,
            tint,
        );
        commands
            .entity(entity)
            .insert(ObjectForms { forms, active });
        spawned += 1;

        if prefab_id == hero_prefab_id && hero_entity.is_none() {
            hero_entity = Some((entity, transform));
        }

        if instance.is_protagonist {
            if !is_commandable_prefab {
                warn!(
                    "scene.dat: Player Character instance {prefab_id:#x} is not commandable; ignoring."
                );
            } else if protagonist_entity.is_none() {
                protagonist_entity = Some((entity, transform));
            } else {
                warn!("scene.dat: multiple Player Character instances flagged; keeping the first.");
            }
        }
    }

    if protagonist_entity.is_none() {
        if let Some(hero) = hero_entity {
            protagonist_entity = Some(hero);
        } else {
            let transform = Transform::from_translation(Vec3::new(0.0, PLAYER_Y, 0.0));
            let instance_id = ObjectId::new_v4();
            let entity = spawn_scene_instance_from_scene(
                commands,
                asset_server,
                assets,
                meshes,
                materials,
                material_cache,
                mesh_cache,
                library,
                hero_prefab_id,
                transform,
                instance_id,
                None,
            );
            commands.entity(entity).insert(ObjectForms {
                forms: vec![hero_prefab_id],
                active: 0,
            });
            spawned += 1;
            protagonist_entity = Some((entity, transform));
        }
    }

    if let Some((entity, transform)) = protagonist_entity {
        commands.entity(entity).insert(Player);
        commands
            .entity(entity)
            .insert(Health::new(PLAYER_MAX_HEALTH, PLAYER_MAX_HEALTH));
        commands.entity(entity).insert(LaserDamageAccum::default());
        commands.entity(entity).insert(PlayerAnimator {
            phase: 0.0,
            last_translation: transform.translation,
        });
    }

    Ok(spawned)
}

pub(crate) fn request_scene_save_on_enter_play(mut saves: MessageWriter<SceneSaveRequest>) {
    saves.write(SceneSaveRequest::new("entered Play mode"));
}

pub(crate) fn scene_autosave_detect_changes(
    mut autosave: ResMut<SceneAutosaveState>,
    added_buildings: Query<Entity, Added<BuildObject>>,
    added_units: Query<Entity, Added<Commandable>>,
    added_players: Query<Entity, Added<Player>>,
    changed_building_transforms: Query<Entity, (With<BuildObject>, Changed<Transform>)>,
    changed_unit_transforms: Query<Entity, (With<Commandable>, Changed<Transform>)>,
    changed_building_prefabs: Query<Entity, (With<BuildObject>, Changed<ObjectPrefabId>)>,
    changed_unit_prefabs: Query<Entity, (With<Commandable>, Changed<ObjectPrefabId>)>,
    changed_building_forms: Query<Entity, (With<BuildObject>, Changed<ObjectForms>)>,
    changed_unit_forms: Query<Entity, (With<Commandable>, Changed<ObjectForms>)>,
    changed_building_tints: Query<Entity, (With<BuildObject>, Changed<ObjectTint>)>,
    changed_unit_tints: Query<Entity, (With<Commandable>, Changed<ObjectTint>)>,
    mut removed: RemovedComponents<BuildObject>,
    mut removed_units: RemovedComponents<Commandable>,
    mut removed_players: RemovedComponents<Player>,
) {
    let removed_any = removed.read().next().is_some();
    let removed_units_any = removed_units.read().next().is_some();
    let removed_players_any = removed_players.read().next().is_some();
    let added_any = added_buildings.iter().next().is_some()
        || added_units.iter().next().is_some()
        || added_players.iter().next().is_some();
    let moved_any = changed_building_transforms.iter().next().is_some()
        || changed_unit_transforms.iter().next().is_some();
    let prefab_any = changed_building_prefabs.iter().next().is_some()
        || changed_unit_prefabs.iter().next().is_some();
    let forms_any = changed_building_forms.iter().next().is_some()
        || changed_unit_forms.iter().next().is_some();
    let tint_any = changed_building_tints.iter().next().is_some()
        || changed_unit_tints.iter().next().is_some();

    if !autosave.primed {
        autosave.primed = true;
        autosave.dirty = false;
        return;
    }

    if removed_any
        || removed_units_any
        || removed_players_any
        || added_any
        || moved_any
        || prefab_any
        || forms_any
        || tint_any
    {
        autosave.dirty = true;
    }
}

pub(crate) fn scene_save_requests(
    mut requests: MessageReader<SceneSaveRequest>,
    objects: Query<
        (
            &Transform,
            &ObjectId,
            &ObjectPrefabId,
            Option<&ObjectTint>,
            Option<&ObjectForms>,
            Option<&Player>,
        ),
        Or<(With<BuildObject>, With<Commandable>)>,
    >,
    config: Res<AppConfig>,
    active: Res<crate::realm::ActiveRealmScene>,
    workspace_ui: Res<crate::workspace_ui::WorkspaceUiState>,
    library: Res<ObjectLibrary>,
    mut autosave: ResMut<SceneAutosaveState>,
) {
    let mut last_reason = None;
    for request in requests.read() {
        last_reason = Some(request.reason);
    }
    let Some(reason) = last_reason else {
        return;
    };

    let path = workspace_scene_dat_path(&config, &active, workspace_ui.tab);
    match save_scene_dat_internal(&objects, &library, &path) {
        Ok(instance_count) => {
            info!(
                "Saved {instance_count} scene instances to {} ({reason}).",
                path.display()
            );
            autosave.dirty = false;
            autosave.timer.reset();
        }
        Err(err) => {
            error!("Failed to write {} ({reason}): {err}", path.display());
            autosave.dirty = true;
        }
    }
}

pub(crate) fn scene_autosave_tick(
    time: Res<Time>,
    objects: Query<
        (
            &Transform,
            &ObjectId,
            &ObjectPrefabId,
            Option<&ObjectTint>,
            Option<&ObjectForms>,
            Option<&Player>,
        ),
        Or<(With<BuildObject>, With<Commandable>)>,
    >,
    config: Res<AppConfig>,
    active: Res<crate::realm::ActiveRealmScene>,
    workspace_ui: Res<crate::workspace_ui::WorkspaceUiState>,
    library: Res<ObjectLibrary>,
    mut autosave: ResMut<SceneAutosaveState>,
) {
    autosave.timer.tick(time.delta());
    if !autosave.timer.just_finished() || !autosave.dirty {
        return;
    }

    let path = workspace_scene_dat_path(&config, &active, workspace_ui.tab);
    match save_scene_dat_internal(&objects, &library, &path) {
        Ok(instance_count) => {
            info!(
                "Auto-saved {instance_count} scene instances to {}.",
                path.display()
            );
            autosave.dirty = false;
        }
        Err(err) => {
            error!("Auto-save failed to write {}: {err}", path.display());
            autosave.dirty = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_object_defs_with_anchors_and_attachments() {
        let child_id = 0x1234_u128;
        let projectile_id = 0xDEAD_u128;
        let def = ObjectDef {
            object_id: 0x9999_u128,
            label: "test_object".into(),
            size: Vec3::new(1.0, 2.0, 3.0),
            ground_origin_y: Some(0.75),
            collider: ColliderProfile::CircleXZ { radius: 1.25 },
            interaction: ObjectInteraction {
                blocks_bullets: true,
                blocks_laser: false,
                movement_block: None,
                supports_standing: false,
            },
            aim: Some(crate::object::registry::AimProfile {
                max_yaw_delta_degrees: Some(90.0),
                components: vec![child_id],
            }),
            mobility: Some(MobilityDef {
                mode: MobilityMode::Ground,
                max_speed: 2.75,
            }),
            anchors: vec![AnchorDef {
                name: "mount".to_string().into(),
                transform: Transform::from_translation(Vec3::new(1.0, 2.0, 3.0))
                    .with_rotation(Quat::from_rotation_y(0.5))
                    .with_scale(Vec3::ONE),
            }],
            parts: vec![ObjectPartDef {
                part_id: Some(0xABCD_u128),
                render_priority: None,
                kind: ObjectPartKind::ObjectRef {
                    object_id: child_id,
                },
                attachment: Some(AttachmentDef {
                    parent_anchor: "mount".to_string().into(),
                    child_anchor: "origin".to_string().into(),
                }),
                animations: vec![
                    PartAnimationSlot {
                        channel: "move".to_string().into(),
                        spec: PartAnimationSpec {
                            driver: PartAnimationDriver::MovePhase,
                            speed_scale: 1.25,
                            time_offset_units: 0.4,
                            clip: PartAnimationDef::Loop {
                                duration_secs: 2.0,
                                keyframes: vec![
                                    PartAnimationKeyframeDef {
                                        time_secs: 0.0,
                                        delta: Transform::IDENTITY,
                                    },
                                    PartAnimationKeyframeDef {
                                        time_secs: 1.0,
                                        delta: Transform::IDENTITY
                                            .with_rotation(Quat::from_rotation_y(0.5)),
                                    },
                                ],
                            },
                        },
                    },
                    PartAnimationSlot {
                        channel: "ambient".to_string().into(),
                        spec: PartAnimationSpec {
                            driver: PartAnimationDriver::Always,
                            speed_scale: 2.0,
                            time_offset_units: 0.0,
                            clip: PartAnimationDef::Spin {
                                axis: Vec3::Z,
                                radians_per_unit: 3.5,
                                axis_space:
                                    crate::object::registry::PartAnimationSpinAxisSpace::ChildLocal,
                            },
                        },
                    },
                ],
                transform: Transform::from_translation(Vec3::new(0.1, 0.2, 0.3))
                    .with_rotation(Quat::from_rotation_x(0.25))
                    .with_scale(Vec3::new(1.0, 2.0, 3.0)),
            }],
            minimap_color: Some(Color::srgb(0.2, 0.3, 0.4)),
            health_bar_offset_y: Some(1.5),
            enemy: None,
            muzzle: None,
            projectile: Some(ProjectileProfile {
                obstacle_rule: ProjectileObstacleRule::LaserBlockers,
                speed: 12.5,
                ttl_secs: 1.25,
                damage: 7,
                spawn_energy_impact: true,
            }),
            attack: Some(UnitAttackProfile {
                kind: UnitAttackKind::RangedProjectile,
                cooldown_secs: 0.9,
                damage: 7,
                anim_window_secs: 0.35,
                melee: None,
                ranged: Some(RangedAttackProfile {
                    projectile_prefab: projectile_id,
                    muzzle: AnchorRef {
                        object_id: child_id,
                        anchor: "muzzle".to_string().into(),
                    },
                }),
            }),
        };

        let dat = def_to_dat(&def);
        let decoded = def_from_dat(&dat).expect("def should decode");

        assert_eq!(decoded.object_id, def.object_id);
        assert_eq!(decoded.label, def.label);
        assert_eq!(decoded.size, def.size);
        assert_eq!(decoded.ground_origin_y, def.ground_origin_y);
        assert!(
            matches!(decoded.aim.as_ref(), Some(aim) if aim.components == vec![child_id] && (aim.max_yaw_delta_degrees.unwrap_or(0.0) - 90.0).abs() < 1e-6)
        );
        assert!(matches!(
            decoded.mobility,
            Some(MobilityDef {
                mode: MobilityMode::Ground,
                max_speed
            }) if (max_speed - 2.75).abs() < 1e-6
        ));

        assert_eq!(decoded.anchors.len(), 1);
        assert_eq!(decoded.anchors[0].name.as_ref(), "mount");
        assert!(
            (decoded.anchors[0].transform.translation - Vec3::new(1.0, 2.0, 3.0)).length_squared()
                < 1e-6
        );
        assert!(
            decoded.anchors[0]
                .transform
                .rotation
                .angle_between(Quat::from_rotation_y(0.5))
                < 1e-4
        );

        assert_eq!(decoded.parts.len(), 1);
        let part = &decoded.parts[0];
        assert_eq!(part.part_id, Some(0xABCD_u128));
        assert!(
            matches!(part.kind, ObjectPartKind::ObjectRef { object_id } if object_id == child_id)
        );
        let attachment = part
            .attachment
            .as_ref()
            .expect("attachment should roundtrip");
        assert_eq!(attachment.parent_anchor.as_ref(), "mount");
        assert_eq!(attachment.child_anchor.as_ref(), "origin");
        assert!((part.transform.translation - Vec3::new(0.1, 0.2, 0.3)).length_squared() < 1e-6);
        assert!(
            part.transform
                .rotation
                .angle_between(Quat::from_rotation_x(0.25))
                < 1e-4
        );
        assert!((part.transform.scale - Vec3::new(1.0, 2.0, 3.0)).length_squared() < 1e-6);

        assert_eq!(part.animations.len(), 2);
        let slot = part
            .animations
            .iter()
            .find(|slot| slot.channel.as_ref() == "move")
            .expect("move animation should roundtrip");
        let animation = &slot.spec;
        assert_eq!(animation.driver, PartAnimationDriver::MovePhase);
        assert!((animation.speed_scale - 1.25).abs() < 1e-6);
        assert!((animation.time_offset_units - 0.4).abs() < 1e-6);
        match &animation.clip {
            PartAnimationDef::Loop {
                duration_secs,
                keyframes,
            } => {
                assert!((duration_secs - 2.0).abs() < 1e-6);
                assert_eq!(keyframes.len(), 2);
                assert!((keyframes[0].time_secs - 0.0).abs() < 1e-6);
                assert!((keyframes[1].time_secs - 1.0).abs() < 1e-6);
                assert!(keyframes[0].delta.rotation.angle_between(Quat::IDENTITY) < 1e-6);
                assert!(
                    keyframes[1]
                        .delta
                        .rotation
                        .angle_between(Quat::from_rotation_y(0.5))
                        < 1e-4
                );
            }
            _ => panic!("expected loop animation"),
        }

        let slot = part
            .animations
            .iter()
            .find(|slot| slot.channel.as_ref() == "ambient")
            .expect("ambient animation should roundtrip");
        let animation = &slot.spec;
        assert_eq!(animation.driver, PartAnimationDriver::Always);
        assert!((animation.speed_scale - 2.0).abs() < 1e-6);
        assert!((animation.time_offset_units - 0.0).abs() < 1e-6);
        match &animation.clip {
            PartAnimationDef::Spin {
                axis,
                radians_per_unit,
                axis_space,
            } => {
                assert!((*axis - Vec3::Z).length_squared() < 1e-6);
                assert!((*radians_per_unit - 3.5).abs() < 1e-6);
                assert_eq!(
                    *axis_space,
                    crate::object::registry::PartAnimationSpinAxisSpace::ChildLocal
                );
            }
            _ => panic!("expected spin animation"),
        }

        let projectile = decoded
            .projectile
            .expect("projectile profile should roundtrip");
        assert!(matches!(
            projectile.obstacle_rule,
            ProjectileObstacleRule::LaserBlockers
        ));
        assert!((projectile.speed - 12.5).abs() < 1e-6);
        assert!((projectile.ttl_secs - 1.25).abs() < 1e-6);
        assert_eq!(projectile.damage, 7);
        assert!(projectile.spawn_energy_impact);

        let attack = decoded.attack.as_ref().expect("attack should roundtrip");
        assert!(matches!(attack.kind, UnitAttackKind::RangedProjectile));
        assert!((attack.cooldown_secs - 0.9).abs() < 1e-6);
        assert!((attack.anim_window_secs - 0.35).abs() < 1e-6);
        assert_eq!(attack.damage, 7);
        assert!(attack.melee.is_none());
        let ranged = attack.ranged.as_ref().expect("ranged attack should exist");
        assert_eq!(ranged.projectile_prefab, projectile_id);
        assert_eq!(ranged.muzzle.object_id, child_id);
        assert_eq!(ranged.muzzle.anchor.as_ref(), "muzzle");
    }

    #[test]
    fn gather_referenced_defs_includes_attack_projectile_prefab() {
        let root_id = 0xF000_0000_0000_0000_0000_0000_0000_0001_u128;
        let muzzle_id = 0xF000_0000_0000_0000_0000_0000_0000_0002_u128;
        let projectile_id = 0xF000_0000_0000_0000_0000_0000_0000_0003_u128;

        let mut library = ObjectLibrary::default();
        library.upsert(ObjectDef {
            object_id: muzzle_id,
            label: "muzzle_component".into(),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: vec![AnchorDef {
                name: "muzzle".to_string().into(),
                transform: Transform::IDENTITY,
            }],
            parts: Vec::new(),
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        });
        library.upsert(ObjectDef {
            object_id: projectile_id,
            label: "projectile".into(),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::CircleXZ { radius: 0.1 },
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: Vec::new(),
            parts: Vec::new(),
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: Some(ProjectileProfile {
                obstacle_rule: ProjectileObstacleRule::BulletsBlockers,
                speed: 10.0,
                ttl_secs: 1.0,
                damage: 1,
                spawn_energy_impact: false,
            }),
            attack: None,
        });
        library.upsert(ObjectDef {
            object_id: root_id,
            label: "root".into(),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::AabbXZ {
                half_extents: Vec2::ONE,
            },
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: Some(MobilityDef {
                mode: MobilityMode::Ground,
                max_speed: 2.0,
            }),
            anchors: Vec::new(),
            parts: vec![ObjectPartDef::object_ref(muzzle_id, Transform::IDENTITY)],
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: Some(UnitAttackProfile {
                kind: UnitAttackKind::RangedProjectile,
                cooldown_secs: 1.0,
                damage: 1,
                anim_window_secs: 0.2,
                melee: None,
                ranged: Some(RangedAttackProfile {
                    projectile_prefab: projectile_id,
                    muzzle: AnchorRef {
                        object_id: muzzle_id,
                        anchor: "muzzle".to_string().into(),
                    },
                }),
            }),
        });

        let ids = gather_referenced_defs(&library, [root_id]);
        assert!(ids.contains(&root_id));
        assert!(ids.contains(&muzzle_id));
        assert!(ids.contains(&projectile_id));
    }
}
