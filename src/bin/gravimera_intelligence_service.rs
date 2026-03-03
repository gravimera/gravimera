use gravimera::intelligence::protocol::*;
use std::collections::{HashMap, HashSet};

const MODULE_DEMO_ORBIT: &str = "demo.orbit.v1";
const MODULE_DEMO_COWARD: &str = "demo.coward.v1";
const MODULE_DEMO_OPPORTUNIST: &str = "demo.opportunist.v1";

#[derive(Debug, Clone)]
struct ServiceConfig {
    bind: String,
    token: Option<String>,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8792".to_string(),
            token: None,
        }
    }
}

fn parse_args() -> ServiceConfig {
    let mut cfg = ServiceConfig::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bind" => {
                if let Some(v) = args.next() {
                    cfg.bind = v;
                } else {
                    eprintln!("`--bind` expects an address like `127.0.0.1:8792`.");
                    std::process::exit(2);
                }
            }
            "--token" => {
                if let Some(v) = args.next() {
                    if !v.trim().is_empty() {
                        cfg.token = Some(v);
                    }
                } else {
                    eprintln!("`--token` expects a bearer token string.");
                    std::process::exit(2);
                }
            }
            "--help" | "-h" => {
                println!(
                    "gravimera_intelligence_service\n\
                     \n\
                     Options:\n\
                       --bind 127.0.0.1:8792   Bind address (default: 127.0.0.1:8792)\n\
                       --token <token>         Require Authorization: Bearer <token>\n"
                );
                std::process::exit(0);
            }
            other => {
                eprintln!("Unknown arg: {other}");
                std::process::exit(2);
            }
        }
    }
    cfg
}

#[derive(Debug, Clone, Copy)]
enum CowardMode {
    Wander,
    Rest,
    Look,
}

impl Default for CowardMode {
    fn default() -> Self {
        Self::Wander
    }
}

#[derive(Debug, Default)]
struct CowardBrain {
    mode: CowardMode,
    mode_until_tick: u64,
    wander_target: Option<[f32; 3]>,
    last_health: Option<i32>,
    dangerous: HashMap<String, u64>,
    last_attacker: Option<String>,
    last_attacked_tick: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
enum OpportunistMode {
    Rest,
    Wander,
}

impl Default for OpportunistMode {
    fn default() -> Self {
        Self::Rest
    }
}

#[derive(Debug, Clone)]
struct CombatTarget {
    id: String,
    last_known_pos: [f32; 3],
    last_known_health: Option<i32>,
    last_known_health_max: Option<i32>,
    last_seen_tick: u64,
}

#[derive(Debug, Default)]
struct OpportunistBrain {
    mode: OpportunistMode,
    mode_until_tick: u64,
    wander_target: Option<[f32; 3]>,
    last_health: Option<i32>,
    health_max_est: Option<i32>,
    target: Option<CombatTarget>,
}

#[derive(Debug)]
enum BrainModuleState {
    DemoOrbit,
    DemoCoward(CowardBrain),
    DemoOpportunist(OpportunistBrain),
}

#[derive(Debug)]
struct BrainInstance {
    brain_instance_id: String,
    realm_id: String,
    scene_id: String,
    unit_instance_id: String,
    module_id: String,
    config: serde_json::Value,
    capabilities: HashSet<String>,
    module_state: BrainModuleState,
}

#[derive(Default)]
struct ServiceState {
    loaded_modules: HashSet<String>,
    brains: HashMap<String, BrainInstance>,
}

fn module_supported(module_id: &str) -> bool {
    let module_id = module_id.trim();
    module_id == MODULE_DEMO_ORBIT
        || module_id == MODULE_DEMO_COWARD
        || module_id == MODULE_DEMO_OPPORTUNIST
}

fn respond_json(request: tiny_http::Request, status: u16, body_json: String) {
    let mut response = tiny_http::Response::from_string(body_json);
    response = response.with_status_code(tiny_http::StatusCode(status));
    if let Ok(header) = tiny_http::Header::from_bytes("Content-Type", "application/json") {
        response = response.with_header(header);
    }
    let _ = request.respond(response);
}

fn bearer_token(request: &tiny_http::Request) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Authorization"))
        .map(|h| h.value.as_str().to_string())
}

fn main() {
    let cfg = parse_args();
    let server = match tiny_http::Server::http(&cfg.bind) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("Failed to bind {}: {err}", cfg.bind);
            std::process::exit(1);
        }
    };

    println!(
        "Intelligence Service listening on http://{} (protocol_version={})",
        server.server_addr(),
        PROTOCOL_VERSION
    );

    let mut state = ServiceState::default();
    for mut request in server.incoming_requests() {
        let method = request.method().as_str().to_string();
        let path_full = request.url().to_string();
        let path = path_full
            .split_once('?')
            .map(|(p, _q)| p.to_string())
            .unwrap_or(path_full);

        if let Some(expected) = cfg.token.as_deref() {
            let header = bearer_token(&request);
            let expected = format!("Bearer {expected}");
            if header.as_deref() != Some(expected.as_str()) {
                respond_json(
                    request,
                    401,
                    serde_json::to_string(&ErrorResponse::new("Unauthorized"))
                        .unwrap_or_else(|_| "{\"ok\":false,\"error\":\"Unauthorized\"}".into()),
                );
                continue;
            }
        }

        let mut body = Vec::new();
        if request.body_length().unwrap_or(0) > 0 {
            if let Err(err) = request.as_reader().read_to_end(&mut body) {
                respond_json(
                    request,
                    400,
                    serde_json::to_string(&ErrorResponse::new(format!(
                        "Failed to read request body: {err}"
                    )))
                    .unwrap_or_else(|_| "{\"ok\":false,\"error\":\"Invalid body\"}".into()),
                );
                continue;
            }
        } else {
            let _ = request.as_reader().read_to_end(&mut body);
        }

        match (method.as_str(), path.as_str()) {
            ("GET", "/v1/health") => {
                let body = HealthResponse {
                    ok: true,
                    name: "gravimera_intelligence_service".into(),
                    version: env!("CARGO_PKG_VERSION").into(),
                    protocol_version: PROTOCOL_VERSION,
                };
                respond_json(request, 200, serde_json::to_string(&body).unwrap());
            }
            ("GET", "/v1/modules") => {
                let modules = vec![BrainModuleInfo {
                    module_id: MODULE_DEMO_ORBIT.into(),
                },
                BrainModuleInfo {
                    module_id: MODULE_DEMO_COWARD.into(),
                },
                BrainModuleInfo {
                    module_id: MODULE_DEMO_OPPORTUNIST.into(),
                }];
                let body = ListModulesResponse {
                    ok: true,
                    protocol_version: PROTOCOL_VERSION,
                    modules,
                };
                respond_json(request, 200, serde_json::to_string(&body).unwrap());
            }
            ("POST", "/v1/load_module") => {
                let req: LoadModuleRequest = match serde_json::from_slice(&body) {
                    Ok(v) => v,
                    Err(err) => {
                        respond_json(
                            request,
                            400,
                            serde_json::to_string(&ErrorResponse::new(format!(
                                "Invalid JSON: {err}"
                            )))
                            .unwrap(),
                        );
                        continue;
                    }
                };
                if req.protocol_version != PROTOCOL_VERSION {
                    respond_json(
                        request,
                        400,
                        serde_json::to_string(&ErrorResponse::new("Unsupported protocol_version"))
                            .unwrap(),
                    );
                    continue;
                }
                let module_id = req.module_descriptor.module_id.trim().to_string();
                if !module_supported(&module_id) {
                    respond_json(
                        request,
                        404,
                        serde_json::to_string(&ErrorResponse::new("Module not found")).unwrap(),
                    );
                    continue;
                }
                state.loaded_modules.insert(module_id.clone());
                let resp = LoadModuleResponse {
                    ok: true,
                    protocol_version: PROTOCOL_VERSION,
                    module_id,
                };
                respond_json(request, 200, serde_json::to_string(&resp).unwrap());
            }
            ("POST", "/v1/spawn") => {
                let req: SpawnBrainInstanceRequest = match serde_json::from_slice(&body) {
                    Ok(v) => v,
                    Err(err) => {
                        respond_json(
                            request,
                            400,
                            serde_json::to_string(&ErrorResponse::new(format!(
                                "Invalid JSON: {err}"
                            )))
                            .unwrap(),
                        );
                        continue;
                    }
                };
                if req.protocol_version != PROTOCOL_VERSION {
                    respond_json(
                        request,
                        400,
                        serde_json::to_string(&ErrorResponse::new("Unsupported protocol_version"))
                            .unwrap(),
                    );
                    continue;
                }
                let module_id = req.module_id.trim().to_string();
                if !module_supported(&module_id) {
                    respond_json(
                        request,
                        404,
                        serde_json::to_string(&ErrorResponse::new("Module not found")).unwrap(),
                    );
                    continue;
                }

                let brain_instance_id = uuid::Uuid::new_v4().to_string();
                let module_state = match module_id.as_str() {
                    MODULE_DEMO_ORBIT => BrainModuleState::DemoOrbit,
                    MODULE_DEMO_COWARD => BrainModuleState::DemoCoward(CowardBrain::default()),
                    MODULE_DEMO_OPPORTUNIST => {
                        BrainModuleState::DemoOpportunist(OpportunistBrain::default())
                    }
                    _ => BrainModuleState::DemoOrbit,
                };
                let instance = BrainInstance {
                    brain_instance_id: brain_instance_id.clone(),
                    realm_id: req.realm_id,
                    scene_id: req.scene_id,
                    unit_instance_id: req.unit_instance_id,
                    module_id,
                    config: req.config,
                    capabilities: req.capabilities.into_iter().collect(),
                    module_state,
                };
                state.brains.insert(brain_instance_id.clone(), instance);
                let resp = SpawnBrainInstanceResponse {
                    ok: true,
                    protocol_version: PROTOCOL_VERSION,
                    brain_instance_id,
                };
                respond_json(request, 200, serde_json::to_string(&resp).unwrap());
            }
            ("POST", "/v1/despawn") => {
                let req: DespawnBrainInstanceRequest = match serde_json::from_slice(&body) {
                    Ok(v) => v,
                    Err(err) => {
                        respond_json(
                            request,
                            400,
                            serde_json::to_string(&ErrorResponse::new(format!(
                                "Invalid JSON: {err}"
                            )))
                            .unwrap(),
                        );
                        continue;
                    }
                };
                if req.protocol_version != PROTOCOL_VERSION {
                    respond_json(
                        request,
                        400,
                        serde_json::to_string(&ErrorResponse::new("Unsupported protocol_version"))
                            .unwrap(),
                    );
                    continue;
                }

                let mut removed = 0u32;
                for id in req.brain_instance_ids {
                    if state.brains.remove(id.trim()).is_some() {
                        removed = removed.saturating_add(1);
                    }
                }
                let resp = DespawnBrainInstanceResponse {
                    ok: true,
                    protocol_version: PROTOCOL_VERSION,
                    despawned: removed,
                };
                respond_json(request, 200, serde_json::to_string(&resp).unwrap());
            }
            ("POST", "/v1/tick_many") => {
                let req: TickManyRequest = match serde_json::from_slice(&body) {
                    Ok(v) => v,
                    Err(err) => {
                        respond_json(
                            request,
                            400,
                            serde_json::to_string(&ErrorResponse::new(format!(
                                "Invalid JSON: {err}"
                            )))
                            .unwrap(),
                        );
                        continue;
                    }
                };
                if req.protocol_version != PROTOCOL_VERSION {
                    respond_json(
                        request,
                        400,
                        serde_json::to_string(&ErrorResponse::new("Unsupported protocol_version"))
                            .unwrap(),
                    );
                    continue;
                }

                let caps = BudgetCaps::default();
                let mut outputs = Vec::with_capacity(req.items.len());
                for item in req.items {
                    let id = item.brain_instance_id;
                    let Some(instance) = state.brains.get_mut(&id) else {
                        outputs.push(TickManyOutput {
                            brain_instance_id: id,
                            tick_output: None,
                            error: Some("Unknown brain_instance_id".into()),
                        });
                        continue;
                    };

                    let mut tick_input = item.tick_input;
                    tick_input.clamp_in_place(caps);

                    let mut out = tick_brain(instance, &tick_input);
                    out.clamp_in_place(caps);

                    outputs.push(TickManyOutput {
                        brain_instance_id: id,
                        tick_output: Some(out),
                        error: None,
                    });
                }

                let resp = TickManyResponse {
                    ok: true,
                    protocol_version: PROTOCOL_VERSION,
                    outputs,
                };
                respond_json(request, 200, serde_json::to_string(&resp).unwrap());
            }
            _ => {
                respond_json(
                    request,
                    404,
                    serde_json::to_string(&ErrorResponse::new("Not found")).unwrap(),
                );
            }
        }
    }
}

fn tick_brain(instance: &mut BrainInstance, input: &TickInput) -> TickOutput {
    let config = &instance.config;
    let capabilities = &instance.capabilities;
    match &mut instance.module_state {
        BrainModuleState::DemoOrbit => tick_demo_orbit(config, capabilities, input),
        BrainModuleState::DemoCoward(state) => tick_demo_coward(config, capabilities, state, input),
        BrainModuleState::DemoOpportunist(state) => {
            tick_demo_opportunist(config, capabilities, state, input)
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn next_f32(&mut self) -> f32 {
        // 24 random bits mapped into [0,1).
        let v = (self.next_u64() >> 40) as u32;
        (v as f32) / ((1u32 << 24) as f32)
    }

    fn gen_range_u32(&mut self, min: u32, max_inclusive: u32) -> u32 {
        if min >= max_inclusive {
            return min;
        }
        let span = max_inclusive - min + 1;
        let v = (self.next_u64() >> 32) as u32;
        min + (v % span)
    }

    fn choose_index(&mut self, len: usize) -> usize {
        if len <= 1 {
            return 0;
        }
        let v = (self.next_u64() >> 32) as u32;
        (v as usize) % len
    }
}

fn config_f32(config: &serde_json::Value, key: &str, default: f32, min: f32, max: f32) -> f32 {
    config
        .get(key)
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
        .unwrap_or(default)
        .clamp(min, max)
}

fn config_u32(config: &serde_json::Value, key: &str, default: u32, min: u32, max: u32) -> u32 {
    config
        .get(key)
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(default)
        .clamp(min, max)
}

fn cap_enabled(capabilities: &HashSet<String>, cap: &str) -> bool {
    capabilities.contains(cap)
}

fn is_tagged(entity: &NearbyEntity, tag: &str) -> bool {
    entity.tags.iter().any(|t| t == tag)
}

fn dist2_xz(rel_pos: [f32; 3]) -> f32 {
    rel_pos[0] * rel_pos[0] + rel_pos[2] * rel_pos[2]
}

fn normalize_xz(dx: f32, dz: f32) -> Option<(f32, f32)> {
    let l2 = dx * dx + dz * dz;
    if !l2.is_finite() || l2 <= 1e-6 {
        return None;
    }
    let inv = 1.0 / l2.sqrt();
    Some((dx * inv, dz * inv))
}

fn add_pos(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn flee_from_rel(
    self_pos: [f32; 3],
    threat_rel_pos: [f32; 3],
    distance_m: f32,
    rng: &mut SplitMix64,
) -> [f32; 3] {
    let away_dx = -threat_rel_pos[0];
    let away_dz = -threat_rel_pos[2];
    let (nx, nz) = normalize_xz(away_dx, away_dz).unwrap_or_else(|| {
        let angle = rng.next_f32() * std::f32::consts::TAU;
        (angle.cos(), angle.sin())
    });
    [
        self_pos[0] + nx * distance_m,
        self_pos[1],
        self_pos[2] + nz * distance_m,
    ]
}

fn random_wander_target(self_pos: [f32; 3], radius_m: f32, rng: &mut SplitMix64) -> [f32; 3] {
    let angle = rng.next_f32() * std::f32::consts::TAU;
    let dist = rng.next_f32().sqrt() * radius_m;
    [
        self_pos[0] + angle.cos() * dist,
        self_pos[1],
        self_pos[2] + angle.sin() * dist,
    ]
}

fn nearest_entity<'a>(
    input: &'a TickInput,
    predicate: impl Fn(&NearbyEntity) -> bool,
) -> Option<(&'a NearbyEntity, f32)> {
    let mut best: Option<(&NearbyEntity, f32)> = None;
    for e in &input.nearby_entities {
        if !predicate(e) {
            continue;
        }
        let d2 = dist2_xz(e.rel_pos);
        if !d2.is_finite() {
            continue;
        }
        best = Some(match best {
            None => (e, d2),
            Some((best_e, best_d2)) => {
                if d2 < best_d2 {
                    (e, d2)
                } else {
                    (best_e, best_d2)
                }
            }
        });
    }
    best
}

fn tick_demo_orbit(
    config: &serde_json::Value,
    capabilities: &HashSet<String>,
    input: &TickInput,
) -> TickOutput {
    // Only emit movement commands if allowed by capabilities.
    if !cap_enabled(capabilities, "brain.move") {
        return TickOutput {
            commands: Vec::new(),
            meta: TickOutputMeta::default(),
        };
    }

    // Config (optional): { "center": [x,z], "radius": f32, "rads_per_tick": f32 }
    let (center_x, center_z) = config
        .get("center")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            if arr.len() != 2 {
                return None;
            }
            Some((
                arr.get(0).and_then(|v| v.as_f64())? as f32,
                arr.get(1).and_then(|v| v.as_f64())? as f32,
            ))
        })
        .unwrap_or((0.0, 0.0));
    let radius = config
        .get("radius")
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
        .unwrap_or(6.0)
        .abs()
        .clamp(0.5, 200.0);
    let rads_per_tick = config
        .get("rads_per_tick")
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
        .unwrap_or(0.05)
        .abs()
        .clamp(0.0, 1.0);

    let a = (input.tick_index as f32) * rads_per_tick;
    let x = center_x + a.cos() * radius;
    let z = center_z + a.sin() * radius;
    let y = input.self_state.pos[1];

    TickOutput {
        commands: vec![BrainCommand::MoveTo {
            pos: [x, y, z],
            valid_until_tick: Some(input.tick_index.saturating_add(10)),
        }],
        meta: TickOutputMeta::default(),
    }
}

fn tick_demo_coward(
    config: &serde_json::Value,
    capabilities: &HashSet<String>,
    state: &mut CowardBrain,
    input: &TickInput,
) -> TickOutput {
    const TICKS_PER_SEC: u64 = 60;
    let tick = input.tick_index;
    let mut rng = SplitMix64::new(input.rng_seed ^ 0xC0B4_12D3_4A6E_9F01);

    let can_move = cap_enabled(capabilities, "brain.move");
    let self_kind = input.self_state.kind.as_str();
    let self_pos = input.self_state.pos;

    let flee_trigger_m = config_f32(config, "flee_trigger_m", 2.8, 0.5, 50.0);
    let flee_distance_m = config_f32(config, "flee_distance_m", 12.0, 2.0, 200.0);
    let attacker_guess_m = config_f32(config, "attacker_guess_m", 6.0, 1.0, 50.0);
    let wander_radius_m = config_f32(config, "wander_radius_m", 8.0, 0.5, 200.0);
    let wander_arrival_m = config_f32(config, "wander_arrival_m", 0.8, 0.1, 10.0);
    let panic_hide_ticks = config_u32(config, "panic_hide_ticks", 600, 0, 60_000) as u64;
    let dangerous_ticks = config_u32(config, "dangerous_ticks", (TICKS_PER_SEC * 60) as u32, 1, 1_000_000) as u64;
    let hide_buffer_m = config_f32(config, "hide_buffer_m", 1.2, 0.1, 20.0);

    // Update / expire danger marks.
    state.dangerous.retain(|_id, until| *until > tick);

    // Detect attack via health drop.
    let attacked = match (input.self_state.health, state.last_health) {
        (Some(now), Some(prev)) => now < prev,
        _ => false,
    };
    state.last_health = input.self_state.health;

    if attacked {
        let attacker = nearest_entity(input, |e| {
            if !is_tagged(e, "unit") {
                return false;
            }
            if self_kind.is_empty() {
                return true;
            }
            e.kind != self_kind
        })
        .and_then(|(e, d2)| {
            let max2 = attacker_guess_m * attacker_guess_m;
            (d2 <= max2).then_some(e)
        });

        if let Some(attacker) = attacker {
            state
                .dangerous
                .insert(attacker.entity_instance_id.clone(), tick.saturating_add(dangerous_ticks));
            state.last_attacker = Some(attacker.entity_instance_id.clone());
        }
        state.last_attacked_tick = Some(tick);
    }

    let recently_attacked = state
        .last_attacked_tick
        .is_some_and(|t0| tick.saturating_sub(t0) < panic_hide_ticks);

    let close_threat = nearest_entity(input, |e| {
        if !is_tagged(e, "unit") {
            return false;
        }
        if !self_kind.is_empty() && e.kind == self_kind {
            return false;
        }
        true
    })
    .and_then(|(e, d2)| {
        let max2 = flee_trigger_m * flee_trigger_m;
        (d2 <= max2).then_some((e, d2))
    });

    let dangerous_visible = nearest_entity(input, |e| {
        is_tagged(e, "unit") && state.dangerous.contains_key(e.entity_instance_id.as_str())
    });

    let threat = close_threat
        .map(|(e, _d2)| e)
        .or_else(|| dangerous_visible.map(|(e, _d2)| e));

    if let Some(threat) = threat {
        let threat_pos = add_pos(self_pos, threat.rel_pos);
        let threat_rel = [threat_pos[0] - self_pos[0], threat_pos[1] - self_pos[1], threat_pos[2] - self_pos[2]];

        let mut goal = None;
        if recently_attacked {
            let building = nearest_entity(input, |e| is_tagged(e, "building"))
                .map(|(e, _d2)| e);
            if let Some(building) = building {
                let building_pos = add_pos(self_pos, building.rel_pos);
                let away_dx = building_pos[0] - threat_pos[0];
                let away_dz = building_pos[2] - threat_pos[2];
                if let Some((nx, nz)) = normalize_xz(away_dx, away_dz) {
                    let half = building
                        .aabb_half_extents
                        .map(|h| h[0].max(h[1]))
                        .unwrap_or(1.0)
                        .max(0.1);
                    let offset = (half + hide_buffer_m).clamp(0.5, 50.0);
                    goal = Some([building_pos[0] + nx * offset, self_pos[1], building_pos[2] + nz * offset]);
                }
            }
        }
        let goal = goal.unwrap_or_else(|| flee_from_rel(self_pos, threat_rel, flee_distance_m, &mut rng));

        let mut commands = Vec::new();
        if can_move {
            commands.push(BrainCommand::MoveTo {
                pos: goal,
                valid_until_tick: Some(tick.saturating_add(30)),
            });
        }
        commands.push(BrainCommand::SleepForTicks { ticks: 6 });
        return TickOutput {
            commands,
            meta: TickOutputMeta::default(),
        };
    }

    // Normal idle behavior: wander / rest / look.
    if tick >= state.mode_until_tick {
        let r = rng.next_f32();
        if r < 0.55 {
            state.mode = CowardMode::Wander;
            state.mode_until_tick = tick.saturating_add(rng.gen_range_u32(120, 240) as u64);
            state.wander_target = Some(random_wander_target(self_pos, wander_radius_m, &mut rng));
        } else if r < 0.85 {
            state.mode = CowardMode::Rest;
            state.mode_until_tick = tick.saturating_add(rng.gen_range_u32(90, 210) as u64);
            state.wander_target = None;
        } else {
            state.mode = CowardMode::Look;
            state.mode_until_tick = tick.saturating_add(rng.gen_range_u32(30, 90) as u64);
            state.wander_target = None;
        }
    }

    let mut commands = Vec::new();
    match state.mode {
        CowardMode::Wander => {
            let arrival2 = wander_arrival_m * wander_arrival_m;
            if let Some(target) = state.wander_target {
                let dx = target[0] - self_pos[0];
                let dz = target[2] - self_pos[2];
                let d2 = dx * dx + dz * dz;
                if !d2.is_finite() || d2 <= arrival2 {
                    state.wander_target = Some(random_wander_target(self_pos, wander_radius_m, &mut rng));
                }
            } else {
                state.wander_target = Some(random_wander_target(self_pos, wander_radius_m, &mut rng));
            }

            if can_move {
                if let Some(target) = state.wander_target {
                    commands.push(BrainCommand::MoveTo {
                        pos: target,
                        valid_until_tick: Some(tick.saturating_add(60)),
                    });
                }
            }
            commands.push(BrainCommand::SleepForTicks { ticks: 18 });
        }
        CowardMode::Rest => {
            let remaining = state.mode_until_tick.saturating_sub(tick);
            let sleep = remaining.min(30).max(6) as u32;
            commands.push(BrainCommand::SleepForTicks { ticks: sleep });
        }
        CowardMode::Look => {
            let remaining = state.mode_until_tick.saturating_sub(tick);
            let sleep = remaining.min(18).max(6) as u32;
            commands.push(BrainCommand::SleepForTicks { ticks: sleep });
        }
    }

    TickOutput {
        commands,
        meta: TickOutputMeta::default(),
    }
}

fn tick_demo_opportunist(
    config: &serde_json::Value,
    capabilities: &HashSet<String>,
    state: &mut OpportunistBrain,
    input: &TickInput,
) -> TickOutput {
    const TICKS_PER_SEC: u64 = 60;
    let tick = input.tick_index;
    let mut rng = SplitMix64::new(input.rng_seed ^ 0x0F70_12A9_84B1_4C5E);

    let can_move = cap_enabled(capabilities, "brain.move");
    let can_combat = cap_enabled(capabilities, "brain.combat");

    let self_kind = input.self_state.kind.as_str();
    let self_pos = input.self_state.pos;

    let wander_radius_m = config_f32(config, "wander_radius_m", 7.0, 0.5, 200.0);
    let wander_arrival_m = config_f32(config, "wander_arrival_m", 0.9, 0.1, 10.0);
    let notice_m = config_f32(config, "notice_m", 7.0, 0.5, 50.0);
    let attack_chance = config_f32(config, "attack_chance", 0.45, 0.0, 1.0);
    let moving_threshold_mps = config_f32(config, "moving_threshold_mps", 0.35, 0.0, 50.0);
    let forget_target_ticks = config_u32(config, "forget_target_ticks", (TICKS_PER_SEC * 10) as u32, 1, 1_000_000) as u64;

    if let Some(max) = input.self_state.health_max {
        state.health_max_est = Some(max.max(1));
    }

    let attacked = match (input.self_state.health, state.last_health) {
        (Some(now), Some(prev)) => now < prev,
        _ => false,
    };
    state.last_health = input.self_state.health;

    let self_current = input.self_state.health.unwrap_or(0).max(0);
    let self_max = input
        .self_state
        .health_max
        .or(state.health_max_est)
        .unwrap_or(self_current.max(1))
        .max(1);

    let quarter = (self_max as f32) * 0.25;
    let health_too_low = (self_current as f32) <= quarter;

    if attacked {
        let attacker = nearest_entity(input, |e| {
            if !is_tagged(e, "unit") {
                return false;
            }
            if !self_kind.is_empty() && e.kind == self_kind {
                return false;
            }
            true
        })
        .map(|(e, _d2)| e);
        if let Some(attacker) = attacker {
            let attacker_pos = add_pos(self_pos, attacker.rel_pos);
            state.target = Some(CombatTarget {
                id: attacker.entity_instance_id.clone(),
                last_known_pos: attacker_pos,
                last_known_health: attacker.health,
                last_known_health_max: attacker.health_max,
                last_seen_tick: tick,
            });
        }
    }

    // Refresh target from current snapshot.
    if let Some(target) = state.target.as_mut() {
        if let Some(seen) = input
            .nearby_entities
            .iter()
            .find(|e| e.entity_instance_id == target.id)
        {
            target.last_known_pos = add_pos(self_pos, seen.rel_pos);
            target.last_known_health = seen.health;
            target.last_known_health_max = seen.health_max;
            target.last_seen_tick = tick;
        } else if tick.saturating_sub(target.last_seen_tick) > forget_target_ticks {
            state.target = None;
        }
    }

    // If we have a target, engage or disengage.
    if let Some(target) = state.target.as_ref() {
        let other_current = target.last_known_health.unwrap_or(0).max(0);
        let other_max = target
            .last_known_health_max
            .unwrap_or(other_current.max(1))
            .max(1);

        let estimated_final = self_current.saturating_sub(other_current);
        let estimated_final_ok = (estimated_final as f32) > quarter;
        let self_fraction = (self_current as f32 / self_max as f32).clamp(0.0, 1.0);
        let other_fraction = (other_current as f32 / other_max as f32).clamp(0.0, 1.0);
        let believe_can_beat = self_fraction >= other_fraction && self_fraction > 0.25;

        if !health_too_low && believe_can_beat && estimated_final_ok && other_current > 0 {
            let mut commands = Vec::new();
            if can_combat {
                commands.push(BrainCommand::AttackTarget {
                    target_id: target.id.clone(),
                    valid_until_tick: Some(tick.saturating_add(30)),
                });
            }
            if can_move {
                commands.push(BrainCommand::MoveTo {
                    pos: target.last_known_pos,
                    valid_until_tick: Some(tick.saturating_add(60)),
                });
            }
            commands.push(BrainCommand::SleepForTicks { ticks: 6 });
            return TickOutput {
                commands,
                meta: TickOutputMeta::default(),
            };
        }

        // Too risky: flee away from the last known target position and drop the target.
        let target_pos = target.last_known_pos;
        state.target = None;
        if can_move && other_current > 0 {
            let threat_rel = [
                target_pos[0] - self_pos[0],
                target_pos[1] - self_pos[1],
                target_pos[2] - self_pos[2],
            ];
            let goal = flee_from_rel(self_pos, threat_rel, 10.0, &mut rng);
            return TickOutput {
                commands: vec![
                    BrainCommand::MoveTo {
                        pos: goal,
                        valid_until_tick: Some(tick.saturating_add(60)),
                    },
                    BrainCommand::SleepForTicks { ticks: 12 },
                ],
                meta: TickOutputMeta::default(),
            };
        }
    }

    // No target: maybe pick a moving nearby unit to attack.
    if !health_too_low {
        let max2 = notice_m * notice_m;
        let mut candidates: Vec<&NearbyEntity> = Vec::new();
        for e in &input.nearby_entities {
            if !is_tagged(e, "unit") {
                continue;
            }
            if !self_kind.is_empty() && e.kind == self_kind {
                continue;
            }
            let d2 = dist2_xz(e.rel_pos);
            if !d2.is_finite() || d2 > max2 {
                continue;
            }
            let speed2 = e.rel_vel[0] * e.rel_vel[0] + e.rel_vel[2] * e.rel_vel[2];
            let speed = speed2.max(0.0).sqrt();
            if speed < moving_threshold_mps {
                continue;
            }
            let Some(other_current) = e.health.map(|h| h.max(0)) else {
                continue;
            };
            let other_max = e
                .health_max
                .unwrap_or(other_current.max(1))
                .max(1);
            let self_fraction = (self_current as f32 / self_max as f32).clamp(0.0, 1.0);
            let other_fraction = (other_current as f32 / other_max as f32).clamp(0.0, 1.0);
            let estimated_final = self_current.saturating_sub(other_current);
            if (estimated_final as f32) <= quarter {
                continue;
            }
            if self_fraction < other_fraction {
                continue;
            }
            candidates.push(e);
        }
        if !candidates.is_empty() && rng.next_f32() < attack_chance {
            let pick = candidates[rng.choose_index(candidates.len())];
            let pick_pos = add_pos(self_pos, pick.rel_pos);
            state.target = Some(CombatTarget {
                id: pick.entity_instance_id.clone(),
                last_known_pos: pick_pos,
                last_known_health: pick.health,
                last_known_health_max: pick.health_max,
                last_seen_tick: tick,
            });
        }
    }

    // Rest/wander loop with ~3/4 rest ratio.
    if tick >= state.mode_until_tick {
        let r = rng.next_f32();
        if r < 0.75 {
            state.mode = OpportunistMode::Rest;
            state.mode_until_tick = tick.saturating_add(rng.gen_range_u32(180, 360) as u64);
            state.wander_target = None;
        } else {
            state.mode = OpportunistMode::Wander;
            state.mode_until_tick = tick.saturating_add(rng.gen_range_u32(120, 240) as u64);
            state.wander_target = Some(random_wander_target(self_pos, wander_radius_m, &mut rng));
        }
    }

    let mut commands = Vec::new();
    match state.mode {
        OpportunistMode::Rest => {
            let remaining = state.mode_until_tick.saturating_sub(tick);
            let sleep = remaining.min(60).max(12) as u32;
            commands.push(BrainCommand::SleepForTicks { ticks: sleep });
        }
        OpportunistMode::Wander => {
            let arrival2 = wander_arrival_m * wander_arrival_m;
            if let Some(target) = state.wander_target {
                let dx = target[0] - self_pos[0];
                let dz = target[2] - self_pos[2];
                let d2 = dx * dx + dz * dz;
                if !d2.is_finite() || d2 <= arrival2 {
                    state.wander_target =
                        Some(random_wander_target(self_pos, wander_radius_m, &mut rng));
                }
            } else {
                state.wander_target = Some(random_wander_target(self_pos, wander_radius_m, &mut rng));
            }

            if can_move {
                if let Some(target) = state.wander_target {
                    commands.push(BrainCommand::MoveTo {
                        pos: target,
                        valid_until_tick: Some(tick.saturating_add(60)),
                    });
                }
            }
            commands.push(BrainCommand::SleepForTicks { ticks: 18 });
        }
    }

    TickOutput {
        commands,
        meta: TickOutputMeta::default(),
    }
}
