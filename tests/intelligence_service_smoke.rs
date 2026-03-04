use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use gravimera::intelligence::protocol::*;

fn http_request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> std::io::Result<(u16, String)> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(1))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    let body = body.unwrap_or("");
    let request = format!(
        "{method} {path} HTTP/1.1\r\n\
         Host: {host}\r\n\
         Connection: close\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {len}\r\n\
         \r\n\
         {body}",
        host = addr,
        len = body.as_bytes().len()
    );
    stream.write_all(request.as_bytes())?;
    stream.flush()?;

    let mut raw = String::new();
    stream.read_to_string(&mut raw)?;

    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let status_line = head.lines().next().unwrap_or("");
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);
    Ok((status, body.to_string()))
}

#[test]
fn intelligence_service_health_spawn_tick_and_despawn() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(v) => v,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("Skipping intelligence service smoke test: bind not permitted ({err}).");
            return;
        }
        Err(err) => panic!("bind ephemeral port: {err}"),
    };
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);

    let bin = env!("CARGO_BIN_EXE_gravimera_intelligence_service");
    let mut child = Command::new(bin)
        .args(["--bind", &format!("127.0.0.1:{port}")])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn gravimera_intelligence_service");

    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match http_request(addr, "GET", "/v1/health", None) {
            Ok((200, body)) if body.contains("\"ok\":true") => break,
            Ok((_status, _body)) => {}
            Err(_) => {}
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("Intelligence service did not become ready in time");
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let load = LoadModuleRequest {
        protocol_version: PROTOCOL_VERSION,
        module_descriptor: BrainModuleDescriptor {
            module_id: "demo.orbit.v1".into(),
        },
    };

    let (status, body) = http_request(addr, "GET", "/v1/modules", None).expect("modules");
    assert_eq!(status, 200, "unexpected modules body: {body}");
    let modules: ListModulesResponse = serde_json::from_str(&body).expect("modules response");
    assert!(
        modules
            .modules
            .iter()
            .any(|m| m.module_id == "demo.orbit.v1"),
        "demo module missing from modules list: {:?}",
        modules.modules
    );

    let (status, body) = http_request(
        addr,
        "POST",
        "/v1/load_module",
        Some(&serde_json::to_string(&load).unwrap()),
    )
    .expect("load_module");
    assert_eq!(status, 200, "unexpected load_module body: {body}");
    let _resp: LoadModuleResponse = serde_json::from_str(&body).expect("load_module response");

    let spawn = SpawnBrainInstanceRequest {
        protocol_version: PROTOCOL_VERSION,
        realm_id: "realm".into(),
        scene_id: "scene".into(),
        unit_instance_id: "unit".into(),
        module_id: "demo.orbit.v1".into(),
        config: serde_json::json!({"center":[0.0,0.0],"radius":6.0,"rads_per_tick":0.1}),
        capabilities: vec!["brain.move".into()],
    };
    let (status, body) = http_request(
        addr,
        "POST",
        "/v1/spawn",
        Some(&serde_json::to_string(&spawn).unwrap()),
    )
    .expect("spawn");
    assert_eq!(status, 200, "unexpected spawn body: {body}");
    let spawn_resp: SpawnBrainInstanceResponse = serde_json::from_str(&body).expect("spawn resp");
    assert!(!spawn_resp.brain_instance_id.trim().is_empty());

    let tick_index = 10u64;
    let tick_input = TickInput {
        realm_id: "realm".into(),
        scene_id: "scene".into(),
        unit_instance_id: "unit".into(),
        dt_ms: 16,
        tick_index,
        rng_seed: 0,
        self_state: SelfState {
            pos: [0.0, 3.0, 0.0],
            yaw: 0.0,
            vel: [0.0, 0.0, 0.0],
            health: None,
            health_max: None,
            stamina: None,
            kind: "thing".into(),
            tags: vec![],
        },
        nearby_entities: vec![],
        events: vec![],
        capabilities: vec!["brain.move".into()],
        meta: TickInputMeta::default(),
    };

    let tick_many = TickManyRequest {
        protocol_version: PROTOCOL_VERSION,
        items: vec![TickManyItem {
            brain_instance_id: spawn_resp.brain_instance_id.clone(),
            tick_input,
        }],
    };
    let (status, body) = http_request(
        addr,
        "POST",
        "/v1/tick_many",
        Some(&serde_json::to_string(&tick_many).unwrap()),
    )
    .expect("tick_many");
    assert_eq!(status, 200, "unexpected tick_many body: {body}");
    let resp: TickManyResponse = serde_json::from_str(&body).expect("tick_many resp");
    assert!(resp.ok);
    assert_eq!(resp.outputs.len(), 1);
    let out = resp.outputs.first().unwrap();
    assert!(out.error.is_none(), "tick_many error: {:?}", out.error);
    let out = out.tick_output.as_ref().expect("missing tick_output");
    assert!(!out.commands.is_empty(), "expected a move command");

    let mut saw_move = false;
    for cmd in &out.commands {
        if let BrainCommand::MoveTo {
            pos,
            valid_until_tick,
        } = cmd
        {
            let a = (tick_index as f32) * 0.1;
            let expected_x = a.cos() * 6.0;
            let expected_z = a.sin() * 6.0;
            assert!((pos[0] - expected_x).abs() < 1e-3, "x mismatch: {pos:?}");
            assert!((pos[2] - expected_z).abs() < 1e-3, "z mismatch: {pos:?}");
            assert!((pos[1] - 3.0).abs() < 1e-6, "y mismatch: {pos:?}");
            assert_eq!(*valid_until_tick, Some(tick_index + 10));
            saw_move = true;
        }
    }
    assert!(
        saw_move,
        "expected BrainCommand::MoveTo, got: {:?}",
        out.commands
    );

    let despawn = DespawnBrainInstanceRequest {
        protocol_version: PROTOCOL_VERSION,
        brain_instance_ids: vec![spawn_resp.brain_instance_id.clone()],
    };
    let (status, body) = http_request(
        addr,
        "POST",
        "/v1/despawn",
        Some(&serde_json::to_string(&despawn).unwrap()),
    )
    .expect("despawn");
    assert_eq!(status, 200, "unexpected despawn body: {body}");
    let resp: DespawnBrainInstanceResponse = serde_json::from_str(&body).expect("despawn resp");
    assert_eq!(resp.despawned, 1);

    let _ = child.kill();
    let _ = child.wait();
}
