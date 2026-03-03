use bevy::prelude::*;
use serde_json::json;
use std::net::SocketAddr;

use crate::config::AppConfig;
use crate::constants::*;
use crate::geometry::safe_abs_scale_y;
use crate::intelligence::protocol::*;
use crate::intelligence::sidecar_client::SidecarClient;
use crate::navigation;
use crate::object::registry::ObjectLibrary;
use crate::types::*;

pub(crate) struct IntelligenceHostPlugin;

impl Plugin for IntelligenceHostPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<IntelligenceHostRuntime>();
        app.add_systems(Startup, intelligence_host_init);
        app.add_systems(
            Startup,
            intelligence_debug_spawn_unit.after(crate::setup::setup_rendered),
        );
        app.add_systems(
            Update,
            intelligence_tick
                .before(crate::rts::execute_move_orders)
                .run_if(in_state(BuildScene::Realm))
                .run_if(intelligence_enabled),
        );
    }
}

#[derive(Resource, Default)]
struct IntelligenceHostRuntime {
    enabled: bool,
    service_addr: Option<SocketAddr>,
    token: Option<String>,
    connected: bool,
    modules_loaded: std::collections::HashSet<String>,
    last_connect_attempt_tick: u64,
}

fn intelligence_enabled(runtime: Res<IntelligenceHostRuntime>) -> bool {
    runtime.enabled
}

#[derive(Component, Clone)]
pub(crate) struct StandaloneBrain {
    pub(crate) module_id: String,
    pub(crate) config: serde_json::Value,
    pub(crate) capabilities: Vec<String>,
    pub(crate) brain_instance_id: Option<String>,
    pub(crate) next_tick_due: u64,
    pub(crate) last_error: Option<String>,
}

impl StandaloneBrain {
    pub(crate) fn demo_orbit() -> Self {
        Self {
            module_id: "demo.orbit.v1".into(),
            config: json!({
                "center": [0.0, 0.0],
                "radius": 10.0,
                "rads_per_tick": 0.05
            }),
            capabilities: vec!["brain.move".into()],
            brain_instance_id: None,
            next_tick_due: 0,
            last_error: None,
        }
    }
}

fn intelligence_host_init(mut runtime: ResMut<IntelligenceHostRuntime>, config: Res<AppConfig>) {
    runtime.enabled = config.intelligence_service_enabled;
    runtime.token = config.intelligence_service_token.clone();
    runtime.connected = false;
    runtime.modules_loaded.clear();
    runtime.last_connect_attempt_tick = 0;

    if !runtime.enabled {
        runtime.service_addr = None;
        return;
    }

    let addr_str = config
        .intelligence_service_addr
        .clone()
        .unwrap_or_else(|| "127.0.0.1:8792".to_string());
    match addr_str.parse::<SocketAddr>() {
        Ok(addr) => runtime.service_addr = Some(addr),
        Err(err) => {
            runtime.enabled = false;
            runtime.service_addr = None;
            warn!("Intelligence service disabled: invalid addr `{addr_str}`: {err}");
        }
    }
}

fn intelligence_debug_spawn_unit(
    mut commands: Commands,
    config: Res<AppConfig>,
    assets: Option<Res<crate::assets::SceneAssets>>,
    asset_server: Option<Res<AssetServer>>,
    library: Res<ObjectLibrary>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut material_cache: ResMut<crate::object::visuals::MaterialCache>,
    mut mesh_cache: ResMut<crate::object::visuals::PrimitiveMeshCache>,
    existing: Query<(), With<StandaloneBrain>>,
) {
    if !config.intelligence_service_enabled || !config.intelligence_service_debug_spawn_unit {
        return;
    }
    if existing.iter().next().is_some() {
        return;
    }

    let Some(assets) = assets else {
        warn!("Intelligence debug spawn: missing SceneAssets (rendered mode only).");
        return;
    };
    let Some(asset_server) = asset_server else {
        warn!("Intelligence debug spawn: missing AssetServer (rendered mode only).");
        return;
    };

    let prefab_id = crate::object::types::characters::hero::object_id();
    let Some(def) = library.get(prefab_id) else {
        warn!("Intelligence debug spawn: missing prefab {prefab_id} in ObjectLibrary.");
        return;
    };

    let size = def.size.abs();
    let radius = (size.x.max(size.z) * 0.5).max(0.4);
    let pos = Vec3::new(8.0, library.ground_origin_y_or_default(prefab_id), 0.0);
    let instance_id = ObjectId::new_v4();
    let transform = Transform::from_translation(pos);

    let mut entity_commands = commands.spawn((
        instance_id,
        ObjectPrefabId(prefab_id),
        Commandable,
        Collider { radius },
        transform,
        Visibility::Inherited,
        StandaloneBrain::demo_orbit(),
    ));

    crate::object::visuals::spawn_object_visuals(
        &mut entity_commands,
        &library,
        &asset_server,
        &assets,
        &mut meshes,
        &mut materials,
        &mut material_cache,
        &mut mesh_cache,
        prefab_id,
        None,
    );
}

fn collect_nav_obstacles(
    objects: &Query<
        (&Transform, &AabbCollider, &BuildDimensions, &ObjectPrefabId),
        With<BuildObject>,
    >,
    library: &ObjectLibrary,
) -> Vec<navigation::NavObstacle> {
    let mut obstacles = Vec::with_capacity(objects.iter().len());
    for (transform, collider, dimensions, prefab_id) in objects.iter() {
        let scale_y = safe_abs_scale_y(transform.scale);
        let origin_y = library.ground_origin_y_or_default(prefab_id.0) * scale_y;
        let bottom_y = transform.translation.y - origin_y;
        let top_y = bottom_y + dimensions.size.y;
        let interaction = library.interaction(prefab_id.0);
        obstacles.push(navigation::NavObstacle {
            movement_block: interaction.movement_block,
            supports_standing: interaction.supports_standing,
            center: Vec2::new(transform.translation.x, transform.translation.z),
            half: collider.half_extents,
            bottom_y,
            top_y,
        });
    }
    obstacles
}

fn compute_rng_seed(
    realm_id: &str,
    scene_id: &str,
    unit_instance_id: &str,
    module_id: &str,
    tick_index: u64,
) -> u64 {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(realm_id.as_bytes());
    h.update(b"\n");
    h.update(scene_id.as_bytes());
    h.update(b"\n");
    h.update(unit_instance_id.as_bytes());
    h.update(b"\n");
    h.update(module_id.as_bytes());
    h.update(b"\n");
    h.update(tick_index.to_le_bytes());
    let digest = h.finalize();
    u64::from_le_bytes(digest[0..8].try_into().unwrap_or([0u8; 8]))
}

fn intelligence_tick(
    mut commands: Commands,
    time: Res<Time>,
    config: Res<AppConfig>,
    library: Res<ObjectLibrary>,
    active: Option<Res<crate::realm::ActiveRealmScene>>,
    mut runtime: ResMut<IntelligenceHostRuntime>,
    mut brains: Query<(Entity, &ObjectId, &Transform, Option<&Health>, &mut StandaloneBrain)>,
    movers: Query<(&Collider, &ObjectPrefabId, Option<&Player>), With<Commandable>>,
    build_objects: Query<
        (&Transform, &AabbCollider, &BuildDimensions, &ObjectPrefabId),
        With<BuildObject>,
    >,
) {
    if !config.intelligence_service_enabled {
        runtime.enabled = false;
        return;
    }

    let Some(addr) = runtime.service_addr else {
        return;
    };
    let client = SidecarClient::new(addr, runtime.token.clone());

    // Cheap, tick-index-like counter: use a monotonic u64 based on frames elapsed.
    // This is sufficient for the MVP; deterministic stepping uses Automation's fixed dt.
    let tick_index = (time.elapsed_secs_f64() * 60.0).floor().max(0.0) as u64;

    if !runtime.connected {
        // Avoid spamming connect attempts every frame.
        if tick_index.saturating_sub(runtime.last_connect_attempt_tick) < 30 {
            return;
        }
        runtime.last_connect_attempt_tick = tick_index;
        match client.health() {
            Ok(resp) if resp.protocol_version == PROTOCOL_VERSION => {
                runtime.connected = true;
                info!(
                    "Intelligence service connected: {} (protocol_version={})",
                    addr, resp.protocol_version
                );
            }
            Ok(resp) => {
                warn!(
                    "Intelligence service protocol mismatch: host={} service={} addr={}",
                    PROTOCOL_VERSION, resp.protocol_version, addr
                );
                return;
            }
            Err(err) => {
                debug!("Intelligence service not ready at {addr}: {err}");
                return;
            }
        }
    }

    let (realm_id, scene_id) = active
        .as_ref()
        .map(|a| (a.realm_id.clone(), a.scene_id.clone()))
        .unwrap_or_else(|| ("default".into(), "default".into()));

    // Ensure modules are loaded (best-effort).
    for (_entity, _object_id, _transform, _health, brain) in brains.iter() {
        if runtime.modules_loaded.contains(&brain.module_id) {
            continue;
        }
        match client.load_module(brain.module_id.as_str()) {
            Ok(_) => {
                runtime.modules_loaded.insert(brain.module_id.clone());
            }
            Err(err) => {
                debug!("Intelligence load_module failed (module_id={}): {err}", brain.module_id);
            }
        }
    }

    // Spawn missing instances.
    for (_entity, object_id, _transform, _health, mut brain) in brains.iter_mut() {
        if brain.brain_instance_id.is_some() {
            continue;
        }
        let req = SpawnBrainInstanceRequest {
            protocol_version: PROTOCOL_VERSION,
            realm_id: realm_id.clone(),
            scene_id: scene_id.clone(),
            unit_instance_id: uuid::Uuid::from_u128(object_id.0).to_string(),
            module_id: brain.module_id.clone(),
            config: brain.config.clone(),
            capabilities: brain.capabilities.clone(),
        };
        match client.spawn(req) {
            Ok(resp) => {
                brain.brain_instance_id = Some(resp.brain_instance_id);
                brain.last_error = None;
            }
            Err(err) => {
                brain.last_error = Some(err);
            }
        }
    }

    let dt_ms = (time.delta_secs().max(0.0) * 1000.0).round() as u32;
    if dt_ms == 0 {
        return;
    }

    let obstacles = collect_nav_obstacles(&build_objects, &library);

    // Build a batched tick request.
    let mut items = Vec::new();
    let mut brain_to_entity = std::collections::HashMap::<String, Entity>::new();
    for (entity, object_id, transform, health, brain) in brains.iter_mut() {
        let Some(brain_instance_id) = brain.brain_instance_id.clone() else {
            continue;
        };
        if tick_index < brain.next_tick_due {
            continue;
        }
        brain_to_entity.insert(brain_instance_id.clone(), entity);

        let unit_instance_id = uuid::Uuid::from_u128(object_id.0).to_string();
        let forward = transform.rotation * Vec3::Z;
        let yaw = forward.x.atan2(forward.z);
        let input = TickInput {
            realm_id: realm_id.clone(),
            scene_id: scene_id.clone(),
            unit_instance_id: unit_instance_id.clone(),
            dt_ms,
            tick_index,
            rng_seed: compute_rng_seed(
                realm_id.as_str(),
                scene_id.as_str(),
                unit_instance_id.as_str(),
                brain.module_id.as_str(),
                tick_index,
            ),
            self_state: SelfState {
                pos: [transform.translation.x, transform.translation.y, transform.translation.z],
                yaw,
                vel: [0.0, 0.0, 0.0],
                health: health.map(|h| h.current),
                stamina: None,
            },
            nearby_entities: Vec::new(),
            events: Vec::new(),
            capabilities: brain.capabilities.clone(),
            meta: TickInputMeta::default(),
        };
        items.push(TickManyItem {
            brain_instance_id,
            tick_input: input,
        });
    }

    if items.is_empty() {
        return;
    }

    let resp = match client.tick_many(TickManyRequest {
        protocol_version: PROTOCOL_VERSION,
        items,
    }) {
        Ok(v) => v,
        Err(err) => {
            debug!("Intelligence tick_many failed: {err}");
            return;
        }
    };

    for out in resp.outputs {
        let Some(entity) = brain_to_entity.get(&out.brain_instance_id).copied() else {
            continue;
        };
        let Ok((collider, prefab_id, is_player)) = movers.get(entity) else {
            continue;
        };
        let Some(mut tick_output) = out.tick_output else {
            if let Some(err) = out.error {
                debug!("Intelligence tick error (brain_instance_id={}): {err}", out.brain_instance_id);
            }
            continue;
        };

        tick_output.clamp_in_place(BudgetCaps::default());
        let mut sleep_for = None;
        for cmd in tick_output.commands {
            match cmd {
                BrainCommand::SleepForTicks { ticks } => {
                    sleep_for = Some(sleep_for.unwrap_or(0).max(ticks));
                }
                BrainCommand::MoveTo { pos, .. } => {
                    let goal = Vec2::new(pos[0], pos[2]);
                    let Ok((_entity, _object_id, transform, _health, _brain)) = brains.get(entity)
                    else {
                        continue;
                    };

                    let scale_y = safe_abs_scale_y(transform.scale);
                    let origin_y = if is_player.is_some() {
                        PLAYER_Y
                    } else {
                        library.ground_origin_y_or_default(prefab_id.0) * scale_y
                    };
                    let current_ground_y = (transform.translation.y - origin_y).max(0.0);
                    let goal_ground_y = (pos[1] - origin_y).max(0.0);

                    let radius = collider.radius.max(0.01);
                    let min = Vec2::splat(-WORLD_HALF_SIZE + radius);
                    let max = Vec2::splat(WORLD_HALF_SIZE - radius);
                    let clamped_goal = goal.clamp(min, max);

                    let start = Vec2::new(transform.translation.x, transform.translation.z);
                    let height = library
                        .size(prefab_id.0)
                        .map(|s| s.y * scale_y)
                        .unwrap_or(HERO_HEIGHT_WORLD * scale_y);

                    let mut order = MoveOrder::default();
                    match library.mobility(prefab_id.0).map(|m| m.mode) {
                        Some(crate::object::registry::MobilityMode::Air) => {
                            order.target = Some(clamped_goal);
                        }
                        Some(crate::object::registry::MobilityMode::Ground) => {
                            let Some(path) = navigation::find_path_height_aware(
                                start,
                                current_ground_y,
                                clamped_goal,
                                goal_ground_y,
                                radius,
                                height,
                                WORLD_HALF_SIZE,
                                NAV_GRID_SIZE,
                                &obstacles,
                            ) else {
                                commands.entity(entity).remove::<MoveOrder>();
                                continue;
                            };
                            let path = navigation::smooth_path_height_aware(
                                start,
                                current_ground_y,
                                path,
                                radius,
                                height,
                                NAV_GRID_SIZE,
                                &obstacles,
                            );
                            order.path = path.into();
                            order.target = Some(clamped_goal);
                        }
                        None => {}
                    }
                    if order.target.is_some() {
                        commands.entity(entity).insert(order);
                    }
                }
                _ => {}
            }
        }

        if let Some(ticks) = sleep_for {
            if let Ok((_entity, _obj_id, _transform, _health, mut brain)) = brains.get_mut(entity) {
                brain.next_tick_due = tick_index.saturating_add(ticks as u64);
            }
        }
    }
}
