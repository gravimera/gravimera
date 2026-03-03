use bevy::prelude::*;
use std::collections::{HashSet, VecDeque};

use crate::constants::*;

#[derive(Component, Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ObjectId(pub(crate) u128);

impl ObjectId {
    pub(crate) fn new_v4() -> Self {
        Self(uuid::Uuid::new_v4().as_u128())
    }
}

#[derive(Component, Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ObjectPrefabId(pub(crate) u128);

/// Per-instance multi-form state (see `docs/gamedesign/37_object_forms_and_transformations.md`).
///
/// Invariants:
/// - `forms.len() >= 1`
/// - `active < forms.len()`
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub(crate) struct ObjectForms {
    pub(crate) forms: Vec<u128>,
    pub(crate) active: usize,
}

impl ObjectForms {
    pub(crate) fn new_single(prefab_id: u128) -> Self {
        Self {
            forms: vec![prefab_id],
            active: 0,
        }
    }

    pub(crate) fn active_prefab_id(&self) -> u128 {
        self.forms
            .get(self.active)
            .copied()
            .unwrap_or_else(|| self.forms.first().copied().unwrap_or(0))
    }

    pub(crate) fn ensure_valid(&mut self, fallback_prefab_id: u128) {
        if self.forms.is_empty() {
            self.forms.push(fallback_prefab_id);
        }
        if self.active >= self.forms.len() {
            self.active = 0;
        }
    }

    /// Appends `prefab_id` if missing and returns its index.
    pub(crate) fn append_dedupe(&mut self, prefab_id: u128) -> usize {
        if let Some(idx) = self.forms.iter().position(|v| *v == prefab_id) {
            return idx;
        }
        self.forms.push(prefab_id);
        self.forms.len().saturating_sub(1)
    }
}

#[derive(Component, Copy, Clone, Debug, PartialEq)]
pub(crate) struct ObjectTint(pub(crate) Color);

#[derive(Component)]
pub(crate) struct Player;

#[derive(Component)]
pub(crate) struct Commandable;

#[derive(Component, Copy, Clone, Debug)]
pub(crate) struct Health {
    pub(crate) current: i32,
    pub(crate) max: i32,
}

impl Health {
    pub(crate) fn new(current: i32, max: i32) -> Self {
        let max = max.max(1);
        let current = current.clamp(0, max);
        Self { current, max }
    }

    pub(crate) fn fraction(&self) -> f32 {
        if self.max > 0 {
            (self.current.max(0) as f32 / self.max as f32).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

/// Marker for units that have died (health <= 0) and should remain as corpses.
///
/// `restore_transform` is used to revive the unit when switching back to Build mode.
#[derive(Component, Clone, Debug)]
pub(crate) struct Died {
    pub(crate) restore_transform: Transform,
}

#[derive(Component, Clone, Debug)]
pub(crate) struct DieMotion {
    pub(crate) started_at_secs: f32,
    pub(crate) duration_secs: f32,
    pub(crate) start: Transform,
    pub(crate) end: Transform,
}

#[derive(Component, Copy, Clone, Debug, Default)]
pub(crate) struct LaserDamageAccum(pub(crate) f32);

#[derive(Component, Copy, Clone, Debug)]
pub(crate) struct ProjectileOwner(pub(crate) Entity);

#[derive(Component)]
pub(crate) struct Enemy {
    pub(crate) speed: f32,
    pub(crate) origin_y: f32,
}

#[derive(Component)]
pub(crate) struct Bullet {
    pub(crate) velocity: Vec3,
    pub(crate) ttl_secs: f32,
}

#[derive(Component)]
pub(crate) struct BulletVisual;

#[derive(Component)]
pub(crate) struct BulletTrailVisual;

#[derive(Component)]
pub(crate) struct EnemyShooter {
    pub(crate) timer: Timer,
    pub(crate) projectile_prefab: u128,
}

#[derive(Component)]
pub(crate) struct EnemyProjectile {
    pub(crate) velocity: Vec3,
    pub(crate) ttl_secs: f32,
}

#[derive(Component)]
pub(crate) struct GundamShooter {
    pub(crate) cooldown_secs: f32,
    pub(crate) shots_left: u8,
}

#[derive(Component)]
pub(crate) struct GundamEnergyBallVisual {
    pub(crate) phase: f32,
}

#[derive(Component)]
pub(crate) struct GundamEnergyArcVisual {
    pub(crate) axis: Vec3,
    pub(crate) phase: f32,
}

#[derive(Component)]
pub(crate) struct Laser {
    pub(crate) ttl_secs: f32,
    pub(crate) direction: Vec3,
}

#[derive(Component, Copy, Clone)]
pub(crate) struct Collider {
    pub(crate) radius: f32,
}

#[derive(Component)]
pub(crate) struct MainCamera;

#[derive(Resource)]
pub(crate) struct CameraZoom {
    pub(crate) t: f32,
}

impl Default for CameraZoom {
    fn default() -> Self {
        Self {
            t: CAMERA_ZOOM_DEFAULT,
        }
    }
}

#[derive(Resource)]
pub(crate) struct CameraYaw {
    pub(crate) yaw: f32,
    pub(crate) initialized: bool,
}

impl Default for CameraYaw {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            initialized: false,
        }
    }
}

#[derive(Resource)]
pub(crate) struct CameraPitch {
    pub(crate) pitch: f32,
}

impl Default for CameraPitch {
    fn default() -> Self {
        Self { pitch: 0.0 }
    }
}

#[derive(Resource)]
pub(crate) struct CameraFocus {
    pub(crate) position: Vec3,
    pub(crate) initialized: bool,
}

impl Default for CameraFocus {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            initialized: false,
        }
    }
}

#[derive(Component)]
pub(crate) struct EdgeScrollIndicatorRoot;

#[derive(Component)]
pub(crate) struct EdgeScrollIndicatorText;

#[derive(Resource, Default)]
pub(crate) struct CommandConsole {
    pub(crate) open: bool,
    pub(crate) buffer: String,
}

#[derive(Component)]
pub(crate) struct CommandConsoleRoot;

#[derive(Component)]
pub(crate) struct CommandConsoleText;

#[derive(Component)]
pub(crate) struct HealthBar {
    pub(crate) root: Entity,
    pub(crate) fill: Entity,
}

#[derive(Component)]
pub(crate) struct HealthBarFill;

#[derive(Component)]
pub(crate) struct FpsCounterText;

#[derive(Component)]
pub(crate) struct MinimapRoot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MinimapMarkerKind {
    Player,
    Enemy,
}

#[derive(Component)]
pub(crate) struct MinimapMarker {
    pub(crate) target: Entity,
    pub(crate) kind: MinimapMarkerKind,
    pub(crate) dir_dot: Option<Entity>,
}

#[derive(Component)]
pub(crate) struct MinimapDirectionDot;

#[derive(Component)]
pub(crate) struct MinimapBuilding {
    pub(crate) target: Entity,
}

#[derive(Component)]
pub(crate) struct MinimapWorldBorderDot {
    pub(crate) index: u16,
}

#[derive(Resource)]
pub(crate) struct MinimapIcons {
    pub(crate) triangle: Handle<Image>,
}

#[derive(Component)]
pub(crate) struct LocomotionClock {
    pub(crate) t: f32,
    pub(crate) distance_m: f32,
    pub(crate) signed_distance_m: f32,
    pub(crate) speed_mps: f32,
    pub(crate) last_translation: Vec3,
}

#[derive(Component, Copy, Clone, Debug, Default)]
pub(crate) struct AttackClock {
    pub(crate) started_at_secs: f32,
    pub(crate) duration_secs: f32,
}

#[derive(Component, Copy, Clone, Debug, Default)]
pub(crate) struct AttackCooldown {
    pub(crate) remaining_secs: f32,
}

/// Per-unit yaw delta (radians) between body facing and attention/aim facing.
///
/// This is driven by `FireControl` target selection and clamped by the unit's aim constraints.
/// A value of `0.0` means "aim aligned with body".
#[derive(Component, Copy, Clone, Debug, Default)]
pub(crate) struct AimYawDelta(pub(crate) f32);

#[derive(Component, Copy, Clone, Debug, Default)]
pub(crate) struct AnimationChannelsActive {
    pub(crate) moving: bool,
    pub(crate) attacking_primary: bool,
}

#[derive(Component, Clone, Debug, Default)]
pub(crate) struct ForcedAnimationChannel {
    pub(crate) channel: String,
}

#[derive(Component)]
pub(crate) struct PlayerAnimator {
    pub(crate) phase: f32,
    pub(crate) last_translation: Vec3,
}

#[derive(Component, Copy, Clone)]
pub(crate) struct PlayerLeg {
    pub(crate) side: f32,
}

#[derive(Component)]
pub(crate) struct PlayerGunRig;

#[derive(Component)]
pub(crate) struct PlayerGunVisual {
    pub(crate) weapon: PlayerWeapon,
}

#[derive(Component)]
pub(crate) struct EnemyAnimator {
    pub(crate) phase: f32,
    pub(crate) last_translation: Vec3,
}

#[derive(Component, Copy, Clone)]
pub(crate) struct EnemyLeg {
    pub(crate) group: f32,
}

#[derive(Component)]
pub(crate) struct DogPounceCooldown {
    pub(crate) remaining_secs: f32,
    pub(crate) was_in_range: bool,
}

#[derive(Component)]
pub(crate) struct DogBiteCooldown {
    pub(crate) remaining_secs: f32,
}

#[derive(Component)]
pub(crate) struct DogPounce {
    pub(crate) start: Vec3,
    pub(crate) end: Vec3,
    pub(crate) elapsed_secs: f32,
    pub(crate) duration_secs: f32,
    pub(crate) arc_height: f32,
    pub(crate) did_damage: bool,
}

#[derive(Component)]
pub(crate) struct ExplosionParticle {
    pub(crate) velocity: Vec3,
    pub(crate) ttl_secs: f32,
    pub(crate) total_secs: f32,
    pub(crate) initial_scale: Vec3,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Hash, States)]
pub(crate) enum GameMode {
    #[default]
    Build,
    Play,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Hash, States)]
pub(crate) enum BuildScene {
    #[default]
    Realm,
    Preview,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum PlayerWeapon {
    #[default]
    Normal,
    Shotgun,
    Laser,
}

impl PlayerWeapon {
    pub(crate) fn label(self) -> &'static str {
        match self {
            PlayerWeapon::Normal => "Normal",
            PlayerWeapon::Shotgun => "Shotgun",
            PlayerWeapon::Laser => "Laser",
        }
    }

    pub(crate) fn is_available(self, shotgun_charges: u32, laser_charges: u32) -> bool {
        match self {
            PlayerWeapon::Normal => true,
            PlayerWeapon::Shotgun => shotgun_charges > 0,
            PlayerWeapon::Laser => laser_charges > 0,
        }
    }
}

#[derive(Resource, Clone, Copy)]
pub(crate) struct PlayerMuzzles {
    pub(crate) normal: f32,
    pub(crate) shotgun: f32,
    pub(crate) laser: f32,
}

impl Default for PlayerMuzzles {
    fn default() -> Self {
        let base_back_offset_z =
            PLAYER_TORSO_DEPTH * 0.5 + PLAYER_GUN_OFFSET_Z + PLAYER_GUN_RIG_FORWARD_OFFSET_Z
                - PLAYER_GUN_TORSO_PULLBACK_Z;
        let gun_length = PLAYER_GUN_LENGTH * HERO_GUN_MODEL_SCALE_MULT;
        let muzzle_forward = base_back_offset_z + gun_length;

        Self {
            normal: muzzle_forward,
            shotgun: muzzle_forward,
            laser: muzzle_forward,
        }
    }
}

impl PlayerMuzzles {
    pub(crate) fn for_weapon(self, weapon: PlayerWeapon) -> f32 {
        match weapon {
            PlayerWeapon::Normal => self.normal,
            PlayerWeapon::Shotgun => self.shotgun,
            PlayerWeapon::Laser => self.laser,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BuildObjectKind {
    Block,
    Fence,
    Tree,
}

impl BuildObjectKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            BuildObjectKind::Block => "Block",
            BuildObjectKind::Fence => "Fence",
            BuildObjectKind::Tree => "Tree",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FenceAxis {
    X,
    Z,
}

impl FenceAxis {
    pub(crate) fn toggle(self) -> Self {
        match self {
            FenceAxis::X => FenceAxis::Z,
            FenceAxis::Z => FenceAxis::X,
        }
    }
}

#[derive(Resource)]
pub(crate) struct BuildState {
    pub(crate) selected: BuildObjectKind,
    pub(crate) placing_active: bool,
    pub(crate) fence_axis: FenceAxis,
    pub(crate) tree_variant: u8,
}

impl Default for BuildState {
    fn default() -> Self {
        Self {
            selected: BuildObjectKind::Block,
            placing_active: false,
            fence_axis: FenceAxis::X,
            // Start at medium-size tree. (scale index 1)
            tree_variant: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BuildPreviewSpec {
    pub(crate) kind: BuildObjectKind,
    pub(crate) fence_axis: FenceAxis,
    pub(crate) tree_variant: u8,
}

#[derive(Resource)]
pub(crate) struct BuildPreview {
    pub(crate) center: Vec3,
    pub(crate) visible: bool,
    pub(crate) spec: Option<BuildPreviewSpec>,
}

impl Default for BuildPreview {
    fn default() -> Self {
        Self {
            center: Vec3::ZERO,
            visible: false,
            spec: None,
        }
    }
}

#[derive(Resource, Default)]
pub(crate) struct SelectionState {
    /// Screen-space drag start (window pixels).
    pub(crate) drag_start: Option<Vec2>,
    /// Screen-space drag end (window pixels).
    pub(crate) drag_end: Option<Vec2>,
    pub(crate) selected: HashSet<Entity>,
}

impl SelectionState {
    pub(crate) fn clear(&mut self) {
        self.drag_start = None;
        self.drag_end = None;
        self.selected.clear();
    }
}

#[derive(Component)]
pub(crate) struct SelectionBoxUi;

#[derive(Component, Default)]
pub(crate) struct MoveOrder {
    pub(crate) path: VecDeque<Vec2>,
    pub(crate) target: Option<Vec2>,
}

impl MoveOrder {
    pub(crate) fn clear(&mut self) {
        self.path.clear();
        self.target = None;
    }
}

#[derive(Component, Clone, Debug)]
pub(crate) struct BrainAttackOrder {
    pub(crate) target: Entity,
    pub(crate) valid_until_tick: Option<u64>,
}

#[derive(Resource, Default)]
pub(crate) struct MoveCommandState {
    pub(crate) marker: Option<Entity>,
}

#[derive(Resource, Default)]
pub(crate) struct SlowMoveMode {
    pub(crate) enabled: bool,
}

#[derive(Component)]
pub(crate) struct BuildPreviewMarker;

#[derive(Component, Copy, Clone)]
pub(crate) struct AabbCollider {
    pub(crate) half_extents: Vec2,
}

#[derive(Component, Copy, Clone)]
pub(crate) struct BuildObject;

/// Provenance tag for concrete instances compiled from a procedural scene layer.
///
/// This is used to support deterministic regeneration: a layer owns its outputs unless an
/// instance is pinned (i.e. unowned).
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub(crate) struct SceneLayerOwner {
    pub(crate) layer_id: String,
}

#[derive(Component, Copy, Clone)]
pub(crate) struct BuildDimensions {
    pub(crate) size: Vec3,
}

#[derive(Resource)]
pub(crate) struct Game {
    pub(crate) score: u32,
    pub(crate) enemy_spawn: Timer,
    pub(crate) fire_cooldown_secs: f32,
    pub(crate) weapon: PlayerWeapon,
    pub(crate) laser_charges: u32,
    pub(crate) shotgun_charges: u32,
    pub(crate) game_over: bool,
}

impl Default for Game {
    fn default() -> Self {
        Self {
            score: 0,
            enemy_spawn: Timer::from_seconds(ENEMY_SPAWN_EVERY_SECS, TimerMode::Repeating),
            fire_cooldown_secs: 0.0,
            weapon: PlayerWeapon::Normal,
            laser_charges: 0,
            shotgun_charges: 0,
            game_over: false,
        }
    }
}

#[derive(Resource, Clone, Copy)]
pub(crate) struct SpawnRatios {
    pub(crate) dog: f32,
    pub(crate) human: f32,
    pub(crate) gundam: f32,
}

#[derive(Resource, Default, Clone, Copy, Debug)]
pub(crate) struct AutoSpawnEnemies(pub(crate) bool);

impl Default for SpawnRatios {
    fn default() -> Self {
        Self::new(0.70, 0.27, 0.03)
    }
}

impl SpawnRatios {
    pub(crate) fn new(dog: f32, human: f32, gundam: f32) -> Self {
        let mut ratios = Self { dog, human, gundam };
        ratios.normalize();
        ratios
    }

    pub(crate) fn normalize(&mut self) {
        self.dog = self.dog.max(0.0);
        self.human = self.human.max(0.0);
        self.gundam = self.gundam.max(0.0);

        let total = self.dog + self.human + self.gundam;
        if total <= 0.0001 {
            self.dog = 0.70;
            self.human = 0.27;
            self.gundam = 0.03;
            return;
        }

        self.dog /= total;
        self.human /= total;
        self.gundam /= total;
    }
}

#[derive(Resource, Default)]
pub(crate) struct KilledEnemiesThisFrame(pub(crate) HashSet<Entity>);

#[derive(Resource, Default)]
pub(crate) struct EnemyKillEffects(pub(crate) Vec<Vec3>);

#[derive(Message, Debug, Clone, Copy)]
pub(crate) struct HealthChangeEvent {
    pub(crate) world_pos: Vec3,
    pub(crate) delta: i32,
    pub(crate) is_hero: bool,
}

#[derive(Resource)]
pub(crate) struct Aim {
    pub(crate) direction: Vec3,
    pub(crate) cursor_hit: Vec3,
    pub(crate) has_cursor_hit: bool,
}

impl Default for Aim {
    fn default() -> Self {
        Self {
            direction: Vec3::Z,
            cursor_hit: Vec3::ZERO,
            has_cursor_hit: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum FireTarget {
    Point(Vec2),
    Unit(Entity),
}

#[derive(Resource, Default)]
pub(crate) struct FireControl {
    pub(crate) active: bool,
    pub(crate) target: Option<FireTarget>,
}

#[derive(Component)]
pub(crate) struct MoveTargetMarker;
