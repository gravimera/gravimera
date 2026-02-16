use bevy::prelude::Vec3;

pub(crate) const WORLD_HALF_SIZE: f32 = 25.0;

// Player entity origin is at the hips (feet on y=0).
pub(crate) const PLAYER_Y: f32 = 0.55;
pub(crate) const PLAYER_SPEED: f32 = 11.0;
pub(crate) const SLOW_MOVE_SPEED_MULTIPLIER: f32 = 1.0 / 3.0;
pub(crate) const PLAYER_RADIUS: f32 = 0.55;
pub(crate) const PLAYER_MAX_HEALTH: i32 = 1000;

pub(crate) const PLAYER_LEG_HEIGHT: f32 = PLAYER_Y;
pub(crate) const PLAYER_LEG_SIZE: f32 = 0.24;
pub(crate) const PLAYER_TORSO_WIDTH: f32 = 0.55;
pub(crate) const PLAYER_TORSO_HEIGHT: f32 = 0.65;
pub(crate) const PLAYER_TORSO_DEPTH: f32 = 0.35;
pub(crate) const PLAYER_HEAD_SIZE: f32 = 0.45;
pub(crate) const PLAYER_ARM_LENGTH: f32 = 0.48;
pub(crate) const PLAYER_ARM_THICK: f32 = 0.20;
pub(crate) const PLAYER_GUN_LENGTH: f32 = 0.55;
pub(crate) const PLAYER_GUN_THICK: f32 = 0.16;
pub(crate) const PLAYER_LEG_OFFSET_X: f32 = 0.15;
pub(crate) const PLAYER_ARM_OFFSET_X: f32 = 0.18;
pub(crate) const PLAYER_GUN_Y: f32 = 0.45;
pub(crate) const PLAYER_GUN_OFFSET_Z: f32 = 0.05;
pub(crate) const PLAYER_GUN_RIG_FORWARD_OFFSET_Z: f32 = PLAYER_GUN_LENGTH / 3.0;
pub(crate) const PLAYER_GUN_TORSO_PULLBACK_Z: f32 = PLAYER_GUN_LENGTH * 0.15;
pub(crate) const PLAYER_LEG_SWING_MAX_RADS: f32 = 0.85;
pub(crate) const PLAYER_LEG_SWING_RADS_PER_SEC: f32 = 12.0;

pub(crate) const DOG_LEG_HEIGHT: f32 = DOG_ORIGIN_Y;
pub(crate) const DOG_LEG_THICK: f32 = 0.12;
pub(crate) const DOG_BODY_WIDTH: f32 = 0.48;
pub(crate) const DOG_BODY_HEIGHT: f32 = 0.30;
pub(crate) const DOG_BODY_LENGTH: f32 = 0.70;
pub(crate) const DOG_HEAD_SIZE: f32 = 0.32;
pub(crate) const DOG_LEG_OFFSET_X: f32 = 0.18;
pub(crate) const DOG_LEG_OFFSET_Z: f32 = 0.22;

pub(crate) const HERO_GUN_MODEL_SCALE_MULT: f32 = 3.0;

pub(crate) const GUNDAM_LEG_HEIGHT: f32 = GUNDAM_ORIGIN_Y;
pub(crate) const GUNDAM_LEG_SIZE: f32 = 0.42;
pub(crate) const GUNDAM_TORSO_WIDTH: f32 = 1.10;
pub(crate) const GUNDAM_TORSO_HEIGHT: f32 = 1.35;
pub(crate) const GUNDAM_TORSO_DEPTH: f32 = 0.65;
pub(crate) const GUNDAM_HEAD_SIZE: f32 = 0.70;
pub(crate) const GUNDAM_ARM_LENGTH: f32 = 0.88;
pub(crate) const GUNDAM_ARM_THICK: f32 = 0.34;
pub(crate) const GUNDAM_GUN_LENGTH: f32 = 1.25;
pub(crate) const GUNDAM_GUN_THICK: f32 = 0.36;
pub(crate) const GUNDAM_GUN_Y: f32 = 0.92;
pub(crate) const GUNDAM_GUN_OFFSET_Z: f32 = 0.10;
pub(crate) const GUNDAM_LEG_OFFSET_X: f32 = 0.28;
pub(crate) const GUNDAM_ARM_OFFSET_X: f32 = 0.38;

pub(crate) const BULLET_SPEED: f32 = 28.0;
pub(crate) const BULLET_TTL_SECS: f32 = 1.5;
pub(crate) const BULLET_RADIUS: f32 = 0.18;
pub(crate) const BULLET_DAMAGE: i32 = 2;
pub(crate) const FIRE_COOLDOWN_SECS: f32 = 0.12;
pub(crate) const BULLET_MESH_LENGTH: f32 = 0.60;
pub(crate) const BULLET_TRAIL_MAX_SECS: f32 = 0.05;

pub(crate) const LASER_DURATION_SECS: f32 = 0.5;
pub(crate) const LASER_RANGE: f32 = WORLD_HALF_SIZE * 2.0;
pub(crate) const LASER_HALF_WIDTH: f32 = 0.45;
pub(crate) const LASER_DAMAGE_PER_SEC: f32 = 10.0;

pub(crate) const SHOTGUN_ARC_HALF_ANGLE_DEGREES: f32 = 15.0;
pub(crate) const SHOTGUN_PELLET_RADIUS: f32 = 0.10;
pub(crate) const SHOTGUN_PELLET_SPEED: f32 = BULLET_SPEED * 1.5;
pub(crate) const SHOTGUN_FIRE_COOLDOWN_SECS: f32 = 0.35;

pub(crate) const ENEMY_SPAWN_RADIUS: f32 = 18.0;
pub(crate) const ENEMY_SPAWN_EVERY_SECS: f32 = 0.9;
pub(crate) const ENEMY_BASE_SPEED: f32 = 4.3;

pub(crate) const DOG_RADIUS: f32 = 0.38;
pub(crate) const HUMAN_RADIUS: f32 = 0.55;
pub(crate) const GUNDAM_RADIUS: f32 = 0.95;

pub(crate) const DOG_ORIGIN_Y: f32 = 0.35;
pub(crate) const HUMAN_ORIGIN_Y: f32 = PLAYER_Y;
pub(crate) const GUNDAM_ORIGIN_Y: f32 = 1.15;

pub(crate) const DOG_BASE_SPEED: f32 = 5.6;
pub(crate) const HUMAN_BASE_SPEED: f32 = ENEMY_BASE_SPEED;
pub(crate) const GUNDAM_BASE_SPEED: f32 = 3.4 / 3.0;

pub(crate) const DOG_HEALTH: i32 = 2;
pub(crate) const HUMAN_HEALTH: i32 = 5;
pub(crate) const GUNDAM_HEALTH: i32 = 20;

pub(crate) const HUMAN_STOP_DISTANCE: f32 = 6.0;
pub(crate) const GUNDAM_STOP_DISTANCE: f32 = HUMAN_STOP_DISTANCE * 2.0;
pub(crate) const GUNDAM_TURN_TO_MOVE_THRESHOLD_RADS: f32 = 10.0 * std::f32::consts::PI / 180.0;
pub(crate) const GUNDAM_MAX_TURN_RATE_RADS_PER_SEC: f32 = 2.6;

pub(crate) const ENEMY_FIRE_EVERY_SECS: f32 = 3.0;
pub(crate) const ENEMY_BULLET_SPEED: f32 = 15.0;
pub(crate) const ENEMY_BULLET_TTL_SECS: f32 = 3.0;
pub(crate) const ENEMY_BULLET_RADIUS: f32 = 0.18;
pub(crate) const ENEMY_BULLET_DAMAGE: i32 = 3;
pub(crate) const ENEMY_BULLET_MESH_RADIUS: f32 = ENEMY_BULLET_RADIUS;
pub(crate) const ENEMY_BULLET_SPOT_RADIUS: f32 = ENEMY_BULLET_RADIUS * 0.33;
pub(crate) const HUMAN_BULLET_RIGHT_HAND_OFFSET: f32 = PLAYER_ARM_OFFSET_X;
pub(crate) const GUNDAM_ENERGY_BALL_SPEED: f32 = 20.0;
pub(crate) const GUNDAM_ENERGY_BALL_TTL_SECS: f32 = 4.0;
pub(crate) const GUNDAM_ENERGY_BALL_RADIUS: f32 = 0.22;
pub(crate) const GUNDAM_ENERGY_BALL_DAMAGE: i32 = 10;
pub(crate) const GUNDAM_ENERGY_BALL_MESH_RADIUS: f32 = 0.32;
pub(crate) const GUNDAM_ENERGY_ARC_COUNT: usize = 6;
pub(crate) const GUNDAM_ENERGY_ARC_RADIUS: f32 = 0.34;
pub(crate) const GUNDAM_ENERGY_ARC_JITTER_RADIUS: f32 = 0.10;
pub(crate) const GUNDAM_ENERGY_BALL_PULSE_HZ: f32 = 2.8;
pub(crate) const GUNDAM_ENERGY_ARC_FLICKER_HZ: f32 = 18.0;
pub(crate) const GUNDAM_ENERGY_IMPACT_PARTICLE_COUNT: usize = 16;
pub(crate) const GUNDAM_ENERGY_IMPACT_TTL_SECS: f32 = 0.45;
pub(crate) const GUNDAM_ENERGY_IMPACT_SPEED: f32 = 8.0;
pub(crate) const GUNDAM_BURST_SHOTS: u8 = 5;
pub(crate) const GUNDAM_BURST_SHOT_INTERVAL_SECS: f32 = 0.12;
pub(crate) const GUNDAM_BURST_CHARGE_SECS: f32 = 2.3;

pub(crate) const EXPLOSION_PARTICLE_COUNT: usize = 18;
pub(crate) const EXPLOSION_TTL_SECS: f32 = 0.55;
pub(crate) const EXPLOSION_SPEED: f32 = 6.0;
pub(crate) const EXPLOSION_GRAVITY: f32 = 18.0;

pub(crate) const DOG_POUNCE_TRIGGER_RANGE: f32 = 3.2;
pub(crate) const DOG_POUNCE_CHANCE: f32 = 0.30;
pub(crate) const DOG_POUNCE_DAMAGE: i32 = 5;
pub(crate) const DOG_POUNCE_SPEED: f32 = 18.0;
pub(crate) const DOG_POUNCE_MIN_DURATION_SECS: f32 = 0.18;
pub(crate) const DOG_POUNCE_MAX_DURATION_SECS: f32 = 0.36;
pub(crate) const DOG_POUNCE_HEIGHT_BASE: f32 = 1.25;
pub(crate) const DOG_POUNCE_HEIGHT_PER_METER: f32 = 0.08;
pub(crate) const DOG_POUNCE_COOLDOWN_SECS: f32 = 1.3;
pub(crate) const DOG_POUNCE_FAIL_COOLDOWN_SECS: f32 = 0.35;

pub(crate) const DOG_BITE_EVERY_SECS: f32 = 3.0;
pub(crate) const DOG_BITE_DAMAGE: i32 = 1;
pub(crate) const DOG_BITE_RANGE_PADDING: f32 = 0.14;

pub(crate) const BLOOD_PARTICLE_COUNT: usize = 14;
pub(crate) const BLOOD_TTL_SECS: f32 = 0.45;
pub(crate) const BLOOD_SPEED: f32 = 6.5;

pub(crate) const ENEMY_LEG_SWING_MAX_RADS: f32 = 0.85;
pub(crate) const ENEMY_LEG_SWING_RADS_PER_SEC: f32 = 12.0;

pub(crate) const HERO_HEIGHT_WORLD: f32 =
    PLAYER_LEG_HEIGHT + PLAYER_TORSO_HEIGHT + PLAYER_HEAD_SIZE;
pub(crate) const DOG_HEIGHT_WORLD: f32 = DOG_LEG_HEIGHT + DOG_BODY_HEIGHT + DOG_HEAD_SIZE;
pub(crate) const GUNDAM_HEIGHT_WORLD: f32 =
    GUNDAM_LEG_HEIGHT + GUNDAM_TORSO_HEIGHT + GUNDAM_HEAD_SIZE;

// Solid blocks only block movement if they rise into the character's upper body.
pub(crate) const CROSS_BLOCK_BLOCKING_HEIGHT_FRACTION: f32 = 2.0 / 3.0;
// Fences block movement only if they rise into the character's upper body.
pub(crate) const CROSS_FENCE_BLOCKING_HEIGHT_FRACTION: f32 = 2.0 / 3.0;

// Units contract:
// - All distances in gameplay code are expressed in world-space meters.
// - Build mode uses a small fixed snap grid for predictable alignment.
//
// NOTE: `BUILD_UNIT_SIZE` is a convenience scale used by some built-in prefabs; it must NOT be
// derived from character sizes (to avoid hidden coupling between gameplay tuning and content).
pub(crate) const BUILD_UNIT_SIZE: f32 = 0.25; // 25 cm
pub(crate) const BUILD_GRID_SIZE: f32 = 0.05; // 5 cm build snap step
pub(crate) const DEFAULT_OBJECT_SIZE_M: f32 = 1.0;

pub(crate) const BUILD_BLOCK_SIZE: Vec3 = Vec3::new(
    3.0 * BUILD_UNIT_SIZE,
    3.0 * BUILD_UNIT_SIZE,
    3.0 * BUILD_UNIT_SIZE,
);

pub(crate) const BUILD_FENCE_LENGTH: f32 = 5.0 * BUILD_UNIT_SIZE;
pub(crate) const BUILD_FENCE_WIDTH: f32 = 1.0 * BUILD_UNIT_SIZE;
pub(crate) const BUILD_FENCE_HEIGHT: f32 = 3.0 * BUILD_UNIT_SIZE;

pub(crate) const BUILD_TREE_BASE_SIZE: Vec3 = Vec3::new(
    3.0 * BUILD_UNIT_SIZE,
    5.0 * BUILD_UNIT_SIZE,
    3.0 * BUILD_UNIT_SIZE,
);
pub(crate) const BUILD_TREE_VARIANT_SCALES: [f32; 3] = [1.0, 1.5, 3.0];

pub(crate) const CAMERA_OFFSET: Vec3 = Vec3::new(0.0, 18.0, 18.0);
pub(crate) const CAMERA_ZOOM_SENSITIVITY: f32 = 0.10;
pub(crate) const CAMERA_ZOOM_DEFAULT: f32 = 0.0;
pub(crate) const CAMERA_ZOOM_MIN: f32 = -1.0;
pub(crate) const CAMERA_ZOOM_MAX: f32 = 1.0;
pub(crate) const CAMERA_YAW_DEADZONE_RADS: f32 = 15.0 * std::f32::consts::PI / 180.0;

// Fully zoomed-in camera uses the same view direction as far zoom, just closer.
// (i.e. scrolling zooms without orbiting/rotating the view.)
pub(crate) const CAMERA_ZOOM_NEAR_SCALE: f32 = 1.0 / 3.0;
pub(crate) const CAMERA_ZOOM_FAR_SCALE: f32 = 8.0;

pub(crate) const DEFAULT_HEADLESS_SECONDS: f32 = 5.0;

// Click-to-move (A* grid) and marker.
pub(crate) const NAV_GRID_SIZE: f32 = 0.50; // 50 cm navigation cells (decoupled from Build snap)
pub(crate) const NAV_HEIGHT_QUANT_SIZE: f32 = 0.25; // 25 cm ground-height quantization for nav
pub(crate) const CLICK_MOVE_WAYPOINT_EPS: f32 = 0.18; // ~18 cm "close enough" for waypoints
pub(crate) const CLICK_MOVE_MAX_TURN_RATE_RADS_PER_SEC: f32 = 10.0;
pub(crate) const COMMANDABLE_MIN_SEPARATION_RADIUS: f32 = PLAYER_RADIUS * 0.6;
pub(crate) const MOVE_TARGET_MARKER_RADIUS: f32 = PLAYER_RADIUS * 0.5;
pub(crate) const MOVE_TARGET_MARKER_HEIGHT: f32 = 0.03;
pub(crate) const MOVE_TARGET_MARKER_Y: f32 = MOVE_TARGET_MARKER_HEIGHT * 0.5 + 0.005;

// RTS-style camera: edge-pan + keyboard rotate.
pub(crate) const CAMERA_EDGE_PAN_MARGIN_PX: f32 = 10.0;
pub(crate) const CAMERA_EDGE_PAN_SPEED_FAR_UNITS_PER_SEC: f32 = 20.0;
pub(crate) const CAMERA_EDGE_PAN_SPEED_NEAR_UNITS_PER_SEC: f32 = 8.0;
pub(crate) const CAMERA_KEY_ROTATE_YAW_RADS_PER_SEC: f32 = 1.8;
pub(crate) const CAMERA_KEY_ROTATE_PITCH_RADS_PER_SEC: f32 = 1.2;
pub(crate) const CAMERA_PITCH_DELTA_MIN_RADS: f32 = -0.6;
pub(crate) const CAMERA_PITCH_DELTA_MAX_RADS: f32 = 0.6;

// UI indicator shown when edge-scroll is active.
pub(crate) const EDGE_SCROLL_INDICATOR_SIZE_PX: f32 = 44.0;
pub(crate) const EDGE_SCROLL_INDICATOR_FONT_SIZE_PX: f32 = 32.0;
pub(crate) const EDGE_SCROLL_INDICATOR_PULSE_RADS_PER_SEC: f32 = 4.0;
pub(crate) const EDGE_SCROLL_INDICATOR_ALPHA_MIN: f32 = 0.35;
pub(crate) const EDGE_SCROLL_INDICATOR_ALPHA_MAX: f32 = 0.95;

pub(crate) const HEALTH_BAR_WIDTH: f32 = 1.4;
pub(crate) const HEALTH_BAR_HEIGHT: f32 = 0.10;
pub(crate) const HEALTH_BAR_DEPTH: f32 = 0.14;
pub(crate) const HEALTH_BAR_FILL_SCALE: f32 = 0.88;
pub(crate) const HEALTH_BAR_Z_OFFSET: f32 = 0.05;
pub(crate) const PLAYER_HEALTH_BAR_OFFSET_Y: f32 = PLAYER_TORSO_HEIGHT + PLAYER_HEAD_SIZE + 0.55;
pub(crate) const DOG_HEALTH_BAR_OFFSET_Y: f32 = DOG_BODY_HEIGHT + DOG_HEAD_SIZE + 0.45;
pub(crate) const GUNDAM_HEALTH_BAR_OFFSET_Y: f32 = GUNDAM_TORSO_HEIGHT + GUNDAM_HEAD_SIZE + 0.70;

pub(crate) const MINIMAP_SIZE_PX: f32 = 180.0;
pub(crate) const MINIMAP_BORDER_PX: f32 = 2.0;
pub(crate) const MINIMAP_MARKER_SIZE_PX: f32 = 14.0;
pub(crate) const MINIMAP_DOT_SIZE_PX: f32 = 6.0;
pub(crate) const MINIMAP_DIR_DOT_SIZE_PX: f32 = 4.0;
pub(crate) const MINIMAP_DIR_OFFSET_PX: f32 = 5.0;
pub(crate) const MINIMAP_PLAYER_FIXED_START_T: f32 = 0.75;
pub(crate) const MINIMAP_WORLD_BORDER_THICKNESS_PX: f32 = 2.0;
pub(crate) const MINIMAP_WORLD_BORDER_DOT_SPACING_PX: f32 = 3.0;
