use bevy::prelude::*;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

pub(crate) fn builtin_object_id(key: &str) -> u128 {
    // Deterministic UUID so builtin prefab ids are stable across runs and machines.
    // Using UUID v5 keeps ids well-formed and avoids manual constant management.
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, key.as_bytes()).as_u128()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum MeshKey {
    UnitCube,
    UnitCylinder,
    UnitCone,
    UnitSphere,
    UnitPlane,
    UnitCapsule,
    UnitConicalFrustum,
    UnitTorus,
    UnitTriangle,
    UnitTetrahedron,
    TreeTrunk,
    TreeCone,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum MaterialKey {
    BuildBlock { index: usize },
    FenceStake,
    FenceStick,
    TreeTrunk { variant: usize },
    TreeMain { variant: usize },
    TreeCrown { variant: usize },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PrimitiveParams {
    Capsule {
        radius: f32,
        half_length: f32,
    },
    ConicalFrustum {
        radius_top: f32,
        radius_bottom: f32,
        height: f32,
    },
    Torus {
        minor_radius: f32,
        major_radius: f32,
    },
}

#[derive(Clone, Debug)]
pub(crate) enum PrimitiveVisualDef {
    Mesh {
        mesh: MeshKey,
        material: MaterialKey,
    },
    Primitive {
        mesh: MeshKey,
        params: Option<PrimitiveParams>,
        color: Color,
        unlit: bool,
    },
}

#[derive(Clone, Debug)]
pub(crate) enum ObjectPartKind {
    ObjectRef { object_id: u128 },
    Primitive { primitive: PrimitiveVisualDef },
    Model { scene: Cow<'static, str> },
}

#[derive(Clone, Debug)]
pub(crate) struct AnchorDef {
    pub(crate) name: Cow<'static, str>,
    pub(crate) transform: Transform,
}

#[derive(Clone, Debug)]
pub(crate) struct AttachmentDef {
    pub(crate) parent_anchor: Cow<'static, str>,
    pub(crate) child_anchor: Cow<'static, str>,
}

#[derive(Clone, Debug)]
pub(crate) struct PartAnimationKeyframeDef {
    pub(crate) time_secs: f32,
    pub(crate) delta: Transform,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum PartAnimationSpinAxisSpace {
    /// Axis is expressed in the attachment join frame (the same frame as `attach_to.offset` and
    /// authored keyframe deltas). For non-attachment parts, this is simply the part's local frame.
    #[default]
    Join,
    /// Axis is expressed in the child object's local frame. For attachment edges, the engine
    /// rebases the spin into the join frame using the child anchor transform.
    ChildLocal,
}

#[derive(Clone, Debug)]
pub(crate) enum PartAnimationDef {
    /// A looping animation. `delta` transforms are applied in the part's local space as:
    /// `animated = base * delta(t)`.
    Loop {
        duration_secs: f32,
        keyframes: Vec<PartAnimationKeyframeDef>,
    },
    /// A one-shot keyframe animation. Playback clamps to the first/last keyframe instead of
    /// wrapping.
    Once {
        duration_secs: f32,
        keyframes: Vec<PartAnimationKeyframeDef>,
    },
    /// A ping-pong keyframe animation: plays forward for `duration_secs`, then backward for
    /// `duration_secs`, repeating.
    PingPong {
        duration_secs: f32,
        keyframes: Vec<PartAnimationKeyframeDef>,
    },
    /// A procedural spin around a local-space axis.
    ///
    /// The driving value is provided by the selected `PartAnimationDriver`:
    /// - `Always`: unit is seconds
    /// - `MoveDistance`: unit is meters traveled (XZ)
    /// - `MovePhase`: unit is meters traveled (XZ) (a generic locomotion phase driver)
    Spin {
        axis: Vec3,
        radians_per_unit: f32,
        axis_space: PartAnimationSpinAxisSpace,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PartAnimationDriver {
    /// Driven by wall-clock time (`Time::elapsed_secs()`).
    Always,
    /// Driven by the owning entity's locomotion phase clock (meters traveled while moving).
    MovePhase,
    /// Driven by the owning entity's cumulative movement distance (XZ).
    MoveDistance,
    /// Driven by the owning entity's last attack event time (seconds since the attack started).
    AttackTime,
    /// Driven by the owning entity's current "action" window (seconds since the action started).
    ActionTime,
}

#[derive(Clone, Debug)]
pub(crate) struct PartAnimationSpec {
    pub(crate) driver: PartAnimationDriver,
    pub(crate) speed_scale: f32,
    /// Constant additive offset applied in the clip's time domain (the same units as
    /// `PartAnimationDef::Loop.duration_secs` / keyframe `time_secs` and the derived driver time).
    ///
    /// This enables deterministic phase offsets (e.g. staggered legs) without duplicating
    /// keyframes.
    pub(crate) time_offset_units: f32,
    pub(crate) clip: PartAnimationDef,
}

#[derive(Clone, Debug)]
pub(crate) struct PartAnimationSlot {
    pub(crate) channel: Cow<'static, str>,
    pub(crate) spec: PartAnimationSpec,
}

#[derive(Clone, Debug)]
pub(crate) struct ObjectPartDef {
    pub(crate) part_id: Option<u128>,
    pub(crate) render_priority: Option<i32>,
    pub(crate) kind: ObjectPartKind,
    pub(crate) attachment: Option<AttachmentDef>,
    pub(crate) animations: Vec<PartAnimationSlot>,
    pub(crate) transform: Transform,
}

impl ObjectPartDef {
    pub(crate) fn object_ref(object_id: u128, transform: Transform) -> Self {
        Self {
            part_id: None,
            render_priority: None,
            kind: ObjectPartKind::ObjectRef { object_id },
            attachment: None,
            animations: Vec::new(),
            transform,
        }
    }

    pub(crate) fn primitive(primitive: PrimitiveVisualDef, transform: Transform) -> Self {
        Self {
            part_id: None,
            render_priority: None,
            kind: ObjectPartKind::Primitive { primitive },
            attachment: None,
            animations: Vec::new(),
            transform,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn model(scene: impl Into<Cow<'static, str>>, transform: Transform) -> Self {
        Self {
            part_id: None,
            render_priority: None,
            kind: ObjectPartKind::Model {
                scene: scene.into(),
            },
            attachment: None,
            animations: Vec::new(),
            transform,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn with_part_id(mut self, part_id: u128) -> Self {
        self.part_id = Some(part_id);
        self
    }

    #[allow(dead_code)]
    pub(crate) fn with_render_priority(mut self, render_priority: i32) -> Self {
        self.render_priority = Some(render_priority);
        self
    }

    pub(crate) fn with_attachment(mut self, attachment: AttachmentDef) -> Self {
        self.attachment = Some(attachment);
        self
    }

    #[allow(dead_code)]
    pub(crate) fn with_animation_slot(
        mut self,
        channel: impl Into<Cow<'static, str>>,
        spec: PartAnimationSpec,
    ) -> Self {
        self.animations.push(PartAnimationSlot {
            channel: channel.into(),
            spec,
        });
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MobilityMode {
    Ground,
    Air,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MobilityDef {
    pub(crate) mode: MobilityMode,
    pub(crate) max_speed: f32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum EnemyShooterProfile {
    Repeating {
        projectile_prefab: u128,
        every_secs: f32,
    },
    Burst {
        projectile_prefab: u128,
        shots_per_burst: u8,
        shot_interval_secs: f32,
        charge_secs: f32,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum EnemyVisualProfile {
    Dog,
    Human,
    Gundam,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TurnProfile {
    pub(crate) max_turn_rate_rads_per_sec: f32,
    pub(crate) turn_to_move_threshold_rads: f32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct EnemyProfile {
    pub(crate) visual: EnemyVisualProfile,
    pub(crate) origin_y: f32,
    pub(crate) base_speed: f32,
    pub(crate) max_health: i32,
    pub(crate) stop_distance: Option<f32>,
    pub(crate) shooter: Option<EnemyShooterProfile>,
    pub(crate) turn: Option<TurnProfile>,
    pub(crate) has_pounce: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MuzzleProfile {
    pub(crate) gun_y: f32,
    pub(crate) torso_depth: f32,
    pub(crate) gun_offset_z: f32,
    pub(crate) gun_length: f32,
    pub(crate) right_hand_offset: f32,
}

impl MuzzleProfile {
    pub(crate) fn muzzle_forward(self) -> f32 {
        self.torso_depth * 0.5 + self.gun_offset_z + self.gun_length
    }

    pub(crate) fn world_muzzle_position(self, transform: &Transform, direction: Vec3) -> Vec3 {
        let base = transform.translation
            + Vec3::new(0.0, self.gun_y, 0.0)
            + direction * self.muzzle_forward();

        if self.right_hand_offset.abs() <= 1e-6 {
            base
        } else {
            let right = Vec3::Y.cross(direction);
            base + right * self.right_hand_offset
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProjectileObstacleRule {
    BulletsBlockers,
    LaserBlockers,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ProjectileProfile {
    pub(crate) obstacle_rule: ProjectileObstacleRule,
    pub(crate) speed: f32,
    pub(crate) ttl_secs: f32,
    pub(crate) damage: i32,
    pub(crate) spawn_energy_impact: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnitAttackKind {
    Melee,
    RangedProjectile,
}

#[derive(Clone, Debug)]
pub(crate) struct AnchorRef {
    pub(crate) object_id: u128,
    pub(crate) anchor: Cow<'static, str>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MeleeAttackProfile {
    pub(crate) range: f32,
    pub(crate) radius: f32,
    pub(crate) arc_degrees: f32,
}

#[derive(Clone, Debug)]
pub(crate) struct RangedAttackProfile {
    pub(crate) projectile_prefab: u128,
    pub(crate) muzzle: AnchorRef,
}

#[derive(Clone, Debug)]
pub(crate) struct AimProfile {
    /// Maximum allowed yaw delta (in degrees) between the unit body direction and its attention
    /// direction. `None` means "unlimited" (can aim freely 360 degrees).
    pub(crate) max_yaw_delta_degrees: Option<f32>,
    /// Object ids of child components that should yaw with the unit's attention direction
    /// (e.g. head, turret, weapon).
    pub(crate) components: Vec<u128>,
}

#[derive(Clone, Debug)]
pub(crate) struct UnitAttackProfile {
    pub(crate) kind: UnitAttackKind,
    pub(crate) cooldown_secs: f32,
    pub(crate) damage: i32,
    pub(crate) anim_window_secs: f32,
    pub(crate) melee: Option<MeleeAttackProfile>,
    pub(crate) ranged: Option<RangedAttackProfile>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum MovementBlockRule {
    /// The object always blocks characters when their collision overlaps in XZ.
    Always,
    /// The object blocks characters only if it rises into the character's upper body.
    /// A character may pass under/through the object if it is entirely above or below
    /// `ground_y + character_height * fraction`.
    UpperBodyFraction(f32),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ObjectInteraction {
    pub(crate) blocks_bullets: bool,
    pub(crate) blocks_laser: bool,
    pub(crate) movement_block: Option<MovementBlockRule>,
    pub(crate) supports_standing: bool,
}

impl ObjectInteraction {
    pub(crate) const fn none() -> Self {
        Self {
            blocks_bullets: false,
            blocks_laser: false,
            movement_block: None,
            supports_standing: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ColliderProfile {
    None,
    CircleXZ { radius: f32 },
    AabbXZ { half_extents: Vec2 },
}

#[derive(Clone, Debug)]
pub(crate) struct ObjectDef {
    pub(crate) object_id: u128,
    pub(crate) label: Cow<'static, str>,
    pub(crate) size: Vec3,
    // Distance from the prefab's local origin to the ground plane when the object is resting on
    // the ground. When absent, the engine assumes the prefab is centered and uses `size.y * 0.5`.
    pub(crate) ground_origin_y: Option<f32>,
    pub(crate) collider: ColliderProfile,
    pub(crate) interaction: ObjectInteraction,
    pub(crate) aim: Option<AimProfile>,
    pub(crate) mobility: Option<MobilityDef>,
    pub(crate) anchors: Vec<AnchorDef>,
    pub(crate) parts: Vec<ObjectPartDef>,
    pub(crate) minimap_color: Option<Color>,
    pub(crate) health_bar_offset_y: Option<f32>,
    pub(crate) enemy: Option<EnemyProfile>,
    pub(crate) muzzle: Option<MuzzleProfile>,
    pub(crate) projectile: Option<ProjectileProfile>,
    pub(crate) attack: Option<UnitAttackProfile>,
}

#[derive(Resource)]
pub(crate) struct ObjectLibrary {
    defs: HashMap<u128, ObjectDef>,
}

impl Default for ObjectLibrary {
    fn default() -> Self {
        let mut library = Self {
            defs: HashMap::new(),
        };
        crate::object::types::register_builtin_objects(&mut library);
        library
    }
}

impl ObjectLibrary {
    pub(crate) fn register(&mut self, def: ObjectDef) {
        let object_id = def.object_id;
        let previous = self.defs.insert(object_id, def);
        if previous.is_some() {
            warn!("ObjectLibrary: duplicate object_id {:#x}", object_id);
        }
    }

    pub(crate) fn upsert(&mut self, def: ObjectDef) {
        self.defs.insert(def.object_id, def);
    }

    pub(crate) fn get(&self, object_id: u128) -> Option<&ObjectDef> {
        self.defs.get(&object_id)
    }

    #[allow(dead_code)]
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&u128, &ObjectDef)> {
        self.defs.iter()
    }

    pub(crate) fn interaction(&self, object_id: u128) -> ObjectInteraction {
        self.get(object_id)
            .map(|def| def.interaction)
            .unwrap_or_else(ObjectInteraction::none)
    }

    pub(crate) fn minimap_color(&self, object_id: u128) -> Option<Color> {
        self.get(object_id).and_then(|def| def.minimap_color)
    }

    pub(crate) fn size(&self, object_id: u128) -> Option<Vec3> {
        self.get(object_id).map(|def| def.size)
    }

    pub(crate) fn ground_origin_y_or_default(&self, object_id: u128) -> f32 {
        let Some(def) = self.get(object_id) else {
            return 0.0;
        };
        if let Some(value) = def.ground_origin_y {
            if value.is_finite() && value >= 0.0 {
                return value;
            }
        }
        if def.size.y.is_finite() {
            def.size.y.abs() * 0.5
        } else {
            0.0
        }
    }

    pub(crate) fn collider(&self, object_id: u128) -> Option<ColliderProfile> {
        self.get(object_id).map(|def| def.collider)
    }

    pub(crate) fn health_bar_offset_y(&self, object_id: u128) -> Option<f32> {
        self.get(object_id).and_then(|def| def.health_bar_offset_y)
    }

    pub(crate) fn enemy(&self, object_id: u128) -> Option<EnemyProfile> {
        self.get(object_id).and_then(|def| def.enemy)
    }

    pub(crate) fn muzzle(&self, object_id: u128) -> Option<MuzzleProfile> {
        self.get(object_id).and_then(|def| def.muzzle)
    }

    pub(crate) fn projectile(&self, object_id: u128) -> Option<ProjectileProfile> {
        self.get(object_id).and_then(|def| def.projectile)
    }

    pub(crate) fn attack(&self, object_id: u128) -> Option<UnitAttackProfile> {
        self.get(object_id).and_then(|def| def.attack.clone())
    }

    pub(crate) fn mobility(&self, object_id: u128) -> Option<MobilityDef> {
        self.get(object_id).and_then(|def| def.mobility)
    }

    pub(crate) fn animation_channels_ordered(&self, object_id: u128) -> Vec<String> {
        fn visit(
            library: &ObjectLibrary,
            object_id: u128,
            visited: &mut HashSet<u128>,
            channels: &mut HashSet<String>,
        ) {
            if !visited.insert(object_id) {
                return;
            }
            let Some(def) = library.get(object_id) else {
                return;
            };
            for part in def.parts.iter() {
                for slot in part.animations.iter() {
                    let ch = slot.channel.as_ref().trim();
                    if !ch.is_empty() {
                        channels.insert(ch.to_string());
                    }
                }
                if let ObjectPartKind::ObjectRef { object_id: child } = &part.kind {
                    visit(library, *child, visited, channels);
                }
            }
        }

        let mut visited: HashSet<u128> = HashSet::new();
        let mut channels: HashSet<String> = HashSet::new();
        visit(self, object_id, &mut visited, &mut channels);

        // Runtime motion algorithms can provide `attack_primary` even when no prefab-authored
        // animation slots exist. Surface it for debug forcing and tooling if the prefab can attack.
        if self
            .get(object_id)
            .and_then(|def| def.attack.as_ref())
            .is_some()
        {
            channels.insert("attack_primary".to_string());
        }

        let mut out: Vec<String> = Vec::new();
        for key in ["idle", "move", "action", "attack_primary"] {
            if channels.remove(key) {
                out.push(key.to_string());
            }
        }
        let mut rest: Vec<String> = channels.into_iter().collect();
        rest.sort();
        out.extend(rest);
        out
    }

    pub(crate) fn animation_channels_ordered_top10(&self, object_id: u128) -> Vec<String> {
        let mut out = self.animation_channels_ordered(object_id);
        out.truncate(10);
        out
    }

    pub(crate) fn channel_uses_move_driver(&self, object_id: u128, channel: &str) -> bool {
        let channel = channel.trim();
        if channel.is_empty() {
            return false;
        }

        fn visit(
            library: &ObjectLibrary,
            object_id: u128,
            visited: &mut HashSet<u128>,
            channel: &str,
        ) -> bool {
            if !visited.insert(object_id) {
                return false;
            }
            let Some(def) = library.get(object_id) else {
                return false;
            };
            for part in def.parts.iter() {
                for slot in part.animations.iter() {
                    if slot.channel.as_ref() != channel {
                        continue;
                    }
                    if matches!(
                        slot.spec.driver,
                        PartAnimationDriver::MovePhase | PartAnimationDriver::MoveDistance
                    ) {
                        return true;
                    }
                }
                if let ObjectPartKind::ObjectRef { object_id: child } = &part.kind {
                    if visit(library, *child, visited, channel) {
                        return true;
                    }
                }
            }
            false
        }

        let mut visited: HashSet<u128> = HashSet::new();
        visit(self, object_id, &mut visited, channel)
    }

    pub(crate) fn channel_attack_duration_secs(
        &self,
        object_id: u128,
        channel: &str,
    ) -> Option<f32> {
        let channel = channel.trim();
        if channel.is_empty() {
            return None;
        }

        if channel == "attack_primary" {
            if let Some(v) = self
                .get(object_id)
                .and_then(|def| def.attack.as_ref())
                .map(|a| a.anim_window_secs)
                .filter(|v| v.is_finite())
                .map(|v| v.abs())
                .filter(|v| *v > 1e-3)
            {
                return Some(v.clamp(0.05, 10.0));
            }
        }

        fn visit(
            library: &ObjectLibrary,
            object_id: u128,
            visited: &mut HashSet<u128>,
            channel: &str,
            best: &mut Option<f32>,
        ) {
            if !visited.insert(object_id) {
                return;
            }
            let Some(def) = library.get(object_id) else {
                return;
            };
            for part in def.parts.iter() {
                for slot in part.animations.iter() {
                    if slot.channel.as_ref() != channel {
                        continue;
                    }
                    if slot.spec.driver != PartAnimationDriver::AttackTime {
                        continue;
                    }

                    let candidate = match &slot.spec.clip {
                        PartAnimationDef::Loop { duration_secs, .. }
                        | PartAnimationDef::Once { duration_secs, .. }
                        | PartAnimationDef::PingPong { duration_secs, .. } => {
                            let speed = slot.spec.speed_scale.max(1e-3);
                            (duration_secs / speed).abs()
                        }
                        PartAnimationDef::Spin { .. } => 1.0,
                    };

                    if candidate.is_finite() && candidate > 1e-3 {
                        *best = Some(best.map_or(candidate, |b| b.max(candidate)));
                    }
                }
                if let ObjectPartKind::ObjectRef { object_id: child } = &part.kind {
                    visit(library, *child, visited, channel, best);
                }
            }
        }

        let mut visited: HashSet<u128> = HashSet::new();
        let mut best: Option<f32> = None;
        visit(self, object_id, &mut visited, channel, &mut best);
        best.map(|v| v.clamp(0.05, 10.0))
    }

    pub(crate) fn channel_action_duration_secs(
        &self,
        object_id: u128,
        channel: &str,
    ) -> Option<f32> {
        let channel = channel.trim();
        if channel.is_empty() {
            return None;
        }

        fn visit(
            library: &ObjectLibrary,
            object_id: u128,
            visited: &mut HashSet<u128>,
            channel: &str,
            best: &mut Option<f32>,
        ) {
            if !visited.insert(object_id) {
                return;
            }
            let Some(def) = library.get(object_id) else {
                return;
            };
            for part in def.parts.iter() {
                for slot in part.animations.iter() {
                    if slot.channel.as_ref() != channel {
                        continue;
                    }
                    if slot.spec.driver != PartAnimationDriver::ActionTime {
                        continue;
                    }

                    let candidate = match &slot.spec.clip {
                        PartAnimationDef::Loop { duration_secs, .. }
                        | PartAnimationDef::Once { duration_secs, .. }
                        | PartAnimationDef::PingPong { duration_secs, .. } => {
                            let speed = slot.spec.speed_scale.max(1e-3);
                            (duration_secs / speed).abs()
                        }
                        PartAnimationDef::Spin { .. } => 1.0,
                    };

                    if candidate.is_finite() && candidate > 1e-3 {
                        *best = Some(best.map_or(candidate, |b| b.max(candidate)));
                    }
                }
                if let ObjectPartKind::ObjectRef { object_id: child } = &part.kind {
                    visit(library, *child, visited, channel, best);
                }
            }
        }

        let mut visited: HashSet<u128> = HashSet::new();
        let mut best: Option<f32> = None;
        visit(self, object_id, &mut visited, channel, &mut best);
        best.map(|v| v.clamp(0.05, 10.0))
    }
}
