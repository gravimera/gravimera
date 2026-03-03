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
use crate::action_log::{ActionLogSource, ActionLogState};
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
            OnEnter(GameMode::Build),
            intelligence_clear_brain_orders_on_enter_build.run_if(in_state(BuildScene::Realm)),
        );
        app.add_systems(
            Update,
            intelligence_attach_default_brains
                .before(intelligence_tick)
                .run_if(in_state(GameMode::Play))
                .run_if(in_state(BuildScene::Realm))
                .run_if(intelligence_enabled),
        );
        app.add_systems(
            Update,
            intelligence_tick
                .before(crate::rts::execute_move_orders)
                .run_if(in_state(GameMode::Play))
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

fn intelligence_clear_brain_orders_on_enter_build(
    mut commands: Commands,
    units: Query<Entity, With<StandaloneBrain>>,
) {
    for entity in units.iter() {
        commands.entity(entity).remove::<MoveOrder>();
        commands.entity(entity).remove::<BrainAttackOrder>();
    }
}

fn intelligence_attach_default_brains(
    mut commands: Commands,
    library: Res<ObjectLibrary>,
    units: Query<
        (Entity, &ObjectPrefabId),
        (
            Added<Commandable>,
            Without<Player>,
            Without<Died>,
            Without<StandaloneBrain>,
        ),
    >,
) {
    for (entity, prefab_id) in units.iter() {
        let (module_id, capabilities): (&str, Vec<String>) = if library.attack(prefab_id.0).is_some()
        {
            (
                "demo.opportunist.v1",
                vec!["brain.move".into(), "brain.combat".into()],
            )
        } else {
            ("demo.coward.v1", vec!["brain.move".into()])
        };

        commands.entity(entity).insert(StandaloneBrain {
            module_id: module_id.into(),
            config: json!({}),
            capabilities,
            brain_instance_id: None,
            next_tick_due: 0,
            last_error: None,
        });
    }
}

fn collect_nav_obstacles(
    objects: &Query<
        (&ObjectId, &Transform, &AabbCollider, &BuildDimensions, &ObjectPrefabId),
        With<BuildObject>,
    >,
    library: &ObjectLibrary,
) -> Vec<navigation::NavObstacle> {
    let mut obstacles = Vec::with_capacity(objects.iter().len());
    for (_object_id, transform, collider, dimensions, prefab_id) in objects.iter() {
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
    mut action_log: ResMut<ActionLogState>,
    mut brains: Query<(Entity, &ObjectId, &Transform, Option<&Health>, &mut StandaloneBrain)>,
    movers: Query<
        (
            &Collider,
            &ObjectPrefabId,
            Option<&Player>,
            Option<&MoveOrder>,
            Option<&BrainAttackOrder>,
        ),
        With<Commandable>,
    >,
    units: Query<
        (
            Entity,
            &ObjectId,
            &Transform,
            &Collider,
            &ObjectPrefabId,
            Option<&Health>,
            Option<&Player>,
            Option<&Enemy>,
            Option<&LocomotionClock>,
        ),
        (Or<(With<Commandable>, With<Enemy>)>, Without<Died>),
    >,
    build_objects: Query<
        (&ObjectId, &Transform, &AabbCollider, &BuildDimensions, &ObjectPrefabId),
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
    let caps = BudgetCaps::default();

    const SENSE_RADIUS_M: f32 = 14.0;
    let sense_r2 = SENSE_RADIUS_M * SENSE_RADIUS_M;

    #[derive(Clone)]
    struct SensedUnit {
        entity: Entity,
        instance_id: String,
        kind: String,
        pos: Vec3,
        vel: Vec3,
        health: Option<i32>,
        health_max: Option<i32>,
        radius: f32,
        tags: Vec<String>,
    }

    #[derive(Clone)]
    struct SensedBuild {
        instance_id: String,
        kind: String,
        pos: Vec3,
        half_extents: Vec2,
        tags: Vec<String>,
    }

    let mut sensed_units: Vec<SensedUnit> = Vec::new();
    let mut instance_to_entity = std::collections::HashMap::<String, Entity>::new();
    for (entity, object_id, transform, collider, prefab_id, health, player, enemy, locomotion) in
        units.iter()
    {
        let instance_id = uuid::Uuid::from_u128(object_id.0).to_string();
        instance_to_entity.insert(instance_id.clone(), entity);

        let kind = uuid::Uuid::from_u128(prefab_id.0).to_string();
        let mut tags = vec!["unit".to_string()];
        if player.is_some() {
            tags.push("player".to_string());
        }
        if enemy.is_some() {
            tags.push("enemy".to_string());
        }

        let speed = locomotion.map(|c| c.speed_mps).unwrap_or(0.0);
        let mut forward = transform.rotation * Vec3::Z;
        forward.y = 0.0;
        let vel = if speed > 0.0 && forward.length_squared() > 1e-6 {
            forward.normalize() * speed
        } else {
            Vec3::ZERO
        };

        sensed_units.push(SensedUnit {
            entity,
            instance_id,
            kind,
            pos: transform.translation,
            vel,
            health: health.map(|h| h.current),
            health_max: health.map(|h| h.max),
            radius: collider.radius,
            tags,
        });
    }

    let mut sensed_builds: Vec<SensedBuild> = Vec::new();
    for (object_id, transform, collider, _dimensions, prefab_id) in build_objects.iter() {
        let instance_id = uuid::Uuid::from_u128(object_id.0).to_string();
        let kind = uuid::Uuid::from_u128(prefab_id.0).to_string();
        sensed_builds.push(SensedBuild {
            instance_id,
            kind,
            pos: transform.translation,
            half_extents: collider.half_extents,
            tags: vec!["build".to_string(), "building".to_string()],
        });
    }

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
        let Ok((_collider, prefab_id, is_player, _existing_move, _existing_attack)) =
            movers.get(entity)
        else {
            continue;
        };

        let self_kind = uuid::Uuid::from_u128(prefab_id.0).to_string();
        let mut self_tags = vec!["unit".to_string()];
        if is_player.is_some() {
            self_tags.push("player".to_string());
        }

        let self_vel = sensed_units
            .iter()
            .find(|u| u.entity == entity)
            .map(|u| u.vel)
            .unwrap_or(Vec3::ZERO);

        let unit_instance_id = uuid::Uuid::from_u128(object_id.0).to_string();
        let self_pos = transform.translation;
        let mut nearby: Vec<(f32, NearbyEntity)> = Vec::new();
        for u in &sensed_units {
            if u.entity == entity {
                continue;
            }
            let delta = u.pos - self_pos;
            let dist2 = delta.x * delta.x + delta.z * delta.z;
            if !dist2.is_finite() || dist2 > sense_r2 {
                continue;
            }

            let rel_vel = u.vel - self_vel;
            nearby.push((
                dist2,
                NearbyEntity {
                    entity_instance_id: u.instance_id.clone(),
                    kind: u.kind.clone(),
                    rel_pos: [delta.x, delta.y, delta.z],
                    rel_vel: [rel_vel.x, rel_vel.y, rel_vel.z],
                    tags: u.tags.clone(),
                    health: u.health,
                    health_max: u.health_max,
                    radius: Some(u.radius.max(0.0)),
                    aabb_half_extents: None,
                },
            ));
        }
        for b in &sensed_builds {
            let delta = b.pos - self_pos;
            let dist2 = delta.x * delta.x + delta.z * delta.z;
            if !dist2.is_finite() || dist2 > sense_r2 {
                continue;
            }
            nearby.push((
                dist2,
                NearbyEntity {
                    entity_instance_id: b.instance_id.clone(),
                    kind: b.kind.clone(),
                    rel_pos: [delta.x, delta.y, delta.z],
                    rel_vel: [0.0, 0.0, 0.0],
                    tags: b.tags.clone(),
                    health: None,
                    health_max: None,
                    radius: None,
                    aabb_half_extents: Some([b.half_extents.x.max(0.0), b.half_extents.y.max(0.0)]),
                },
            ));
        }
        nearby.sort_by(|(a_dist2, a), (b_dist2, b)| {
            a_dist2
                .partial_cmp(b_dist2)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.entity_instance_id.cmp(&b.entity_instance_id))
        });
        let mut nearby_entities: Vec<NearbyEntity> =
            nearby.into_iter().map(|(_d2, e)| e).collect();
        let dropped_nearby = nearby_entities
            .len()
            .saturating_sub(caps.max_nearby_entities as usize);
        if nearby_entities.len() > caps.max_nearby_entities as usize {
            nearby_entities.truncate(caps.max_nearby_entities as usize);
        }

        brain_to_entity.insert(brain_instance_id.clone(), entity);

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
                vel: [self_vel.x, self_vel.y, self_vel.z],
                health: health.map(|h| h.current),
                health_max: health.map(|h| h.max),
                stamina: None,
                kind: self_kind,
                tags: self_tags,
            },
            nearby_entities: nearby_entities,
            events: Vec::new(),
            capabilities: brain.capabilities.clone(),
            meta: TickInputMeta {
                nearby_entities_dropped: dropped_nearby as u32,
                events_dropped: 0,
            },
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
                    let Ok((collider, prefab_id, is_player, existing_move, _existing_attack)) =
                        movers.get(entity)
                    else {
                        continue;
                    };
                    let goal = Vec2::new(pos[0], pos[2]);
                    let Ok((_entity, object_id, transform, _health, brain)) = brains.get(entity)
                    else {
                        continue;
                    };
                    let previous_target = existing_move.and_then(|order| order.target);

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

                        let should_log = previous_target
                            .map(|prev| prev.distance(clamped_goal) >= 0.8)
                            .unwrap_or(true);
                        if should_log {
                            let label = library
                                .get(prefab_id.0)
                                .map(|def| def.label.as_ref())
                                .unwrap_or("unit");
                            let module_id = brain.module_id.as_str();
                            let short_id = (object_id.0 & 0xffff_ffff) as u32;
                            action_log.push(
                                time.elapsed_secs(),
                                ActionLogSource::Brain,
                                format!(
                                    "{label}#{short_id:08x} ({module_id}) move → ({:.1}, {:.1})",
                                    clamped_goal.x, clamped_goal.y
                                ),
                            );
                        }
                    }
                }
                BrainCommand::AttackTarget {
                    target_id,
                    valid_until_tick,
                } => {
                    let Ok((_entity, object_id, _transform, _health, brain)) = brains.get(entity)
                    else {
                        continue;
                    };
                    if !brain.capabilities.iter().any(|c| c == "brain.combat") {
                        continue;
                    }
                    if let Some(valid_until_tick) = valid_until_tick {
                        if tick_index > valid_until_tick {
                            continue;
                        }
                    }

                    let target_id = target_id.trim();
                    let Some(target_entity) = instance_to_entity.get(target_id).copied() else {
                        continue;
                    };
                    if target_entity == entity {
                        continue;
                    }

                    let mut should_log = true;
                    if let Ok((_collider, prefab_id, _player, _move, existing_attack)) =
                        movers.get(entity)
                    {
                        if existing_attack.is_some_and(|o| o.target == target_entity) {
                            should_log = false;
                        }

                        if should_log {
                            let attacker_label = library
                                .get(prefab_id.0)
                                .map(|def| def.label.as_ref())
                                .unwrap_or("unit");
                            let attacker_short_id = (object_id.0 & 0xffff_ffff) as u32;
                            let target_label = units
                                .get(target_entity)
                                .ok()
                                .and_then(|(_e, _id, _t, _c, prefab, _h, _p, _en, _lc)| {
                                    library.get(prefab.0).map(|def| def.label.as_ref())
                                })
                                .unwrap_or("unit");
                            action_log.push(
                                time.elapsed_secs(),
                                ActionLogSource::Brain,
                                format!(
                                    "{attacker_label}#{attacker_short_id:08x} ({}) attack → {target_label}",
                                    brain.module_id.as_str()
                                ),
                            );
                        }
                    }

                    commands.entity(entity).insert(crate::types::BrainAttackOrder {
                        target: target_entity,
                        valid_until_tick,
                    });
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
