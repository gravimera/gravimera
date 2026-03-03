use gravimera::intelligence::protocol::*;
use std::collections::{HashMap, HashSet};

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

#[derive(Debug, Clone)]
struct BrainInstance {
    brain_instance_id: String,
    realm_id: String,
    scene_id: String,
    unit_instance_id: String,
    module_id: String,
    config: serde_json::Value,
    capabilities: HashSet<String>,
}

#[derive(Default)]
struct ServiceState {
    loaded_modules: HashSet<String>,
    brains: HashMap<String, BrainInstance>,
}

fn module_supported(module_id: &str) -> bool {
    matches!(module_id.trim(), "demo.orbit.v1")
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
                let instance = BrainInstance {
                    brain_instance_id: brain_instance_id.clone(),
                    realm_id: req.realm_id,
                    scene_id: req.scene_id,
                    unit_instance_id: req.unit_instance_id,
                    module_id,
                    config: req.config,
                    capabilities: req.capabilities.into_iter().collect(),
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
                    let Some(instance) = state.brains.get(&id) else {
                        outputs.push(TickManyOutput {
                            brain_instance_id: id,
                            tick_output: None,
                            error: Some("Unknown brain_instance_id".into()),
                        });
                        continue;
                    };

                    let mut tick_input = item.tick_input;
                    tick_input.clamp_in_place(caps);

                    let mut out = tick_demo_orbit(instance, &tick_input);
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

fn tick_demo_orbit(instance: &BrainInstance, input: &TickInput) -> TickOutput {
    // Only emit movement commands if allowed by capabilities.
    if !instance.capabilities.contains("brain.move") {
        return TickOutput {
            commands: Vec::new(),
            meta: TickOutputMeta::default(),
        };
    }

    // Config (optional): { "center": [x,z], "radius": f32, "rads_per_tick": f32 }
    let (center_x, center_z) = instance
        .config
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
    let radius = instance
        .config
        .get("radius")
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
        .unwrap_or(6.0)
        .abs()
        .clamp(0.5, 200.0);
    let rads_per_tick = instance
        .config
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
