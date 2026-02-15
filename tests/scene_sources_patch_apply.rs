use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn http_request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> std::io::Result<(u16, String)> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(1))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;

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

fn make_temp_dir(prefix: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    dir.push(format!("{prefix}_{pid}_{nanos}"));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn copy_dir_recursive(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create dst dir");
    for entry in std::fs::read_dir(from).expect("read_dir") {
        let entry = entry.expect("dir entry");
        let src_path = entry.path();
        let dst_path = to.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path);
        } else {
            let bytes = std::fs::read(&src_path).expect("read file");
            std::fs::write(&dst_path, bytes).expect("write file");
        }
    }
}

struct ChildGuard {
    child: std::process::Child,
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_for_health(addr: SocketAddr) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match http_request(addr, "GET", "/v1/health", None) {
            Ok((200, body)) if body.contains("\"ok\":true") => break,
            _ => {}
        }
        if Instant::now() >= deadline {
            panic!("Automation API did not become ready in time");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn import_sources(addr: SocketAddr, src_dir: &Path) {
    let import_body = serde_json::json!({
        "src_dir": src_dir.display().to_string(),
    })
    .to_string();
    let (status, body) = http_request(addr, "POST", "/v1/scene_sources/import", Some(&import_body))
        .expect("import request");
    assert_eq!(status, 200, "import failed: status={status} body={body}");
}

fn compile_layers(addr: SocketAddr) {
    let (status, body) =
        http_request(addr, "POST", "/v1/scene_sources/compile", Some("{}")).expect("compile");
    assert_eq!(status, 200, "compile failed: status={status} body={body}");
}

fn get_signature(addr: SocketAddr) -> serde_json::Value {
    let (status, body) =
        http_request(addr, "GET", "/v1/scene_sources/signature", None).expect("signature");
    assert_eq!(status, 200, "signature failed: status={status} body={body}");
    serde_json::from_str(&body).expect("signature json")
}

fn patch_validate(
    addr: SocketAddr,
    scorecard: serde_json::Value,
    patch: serde_json::Value,
) -> serde_json::Value {
    let body = serde_json::json!({ "scorecard": scorecard, "patch": patch }).to_string();
    let (status, body) = http_request(
        addr,
        "POST",
        "/v1/scene_sources/patch_validate",
        Some(&body),
    )
    .expect("patch_validate");
    assert_eq!(
        status, 200,
        "patch_validate failed: status={status} body={body}"
    );
    serde_json::from_str(&body).expect("patch_validate json")
}

fn patch_apply(
    addr: SocketAddr,
    scorecard: serde_json::Value,
    patch: serde_json::Value,
) -> serde_json::Value {
    let body = serde_json::json!({ "scorecard": scorecard, "patch": patch }).to_string();
    let (status, body) =
        http_request(addr, "POST", "/v1/scene_sources/patch_apply", Some(&body)).expect("apply");
    assert_eq!(
        status, 200,
        "patch_apply failed: status={status} body={body}"
    );
    serde_json::from_str(&body).expect("patch_apply json")
}

fn contains_path(list: &serde_json::Value, path: &str) -> bool {
    list.as_array()
        .map(|items| items.iter().any(|v| v.as_str() == Some(path)))
        .unwrap_or(false)
}

#[test]
fn patch_apply_is_idempotent_and_recompiles_deterministically() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(v) => v,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("Skipping patch apply test: bind not permitted ({err}).");
            return;
        }
        Err(err) => panic!("bind ephemeral port: {err}"),
    };
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);

    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();

    let bin = env!("CARGO_BIN_EXE_gravimera");
    let temp_root = make_temp_dir("gravimera_scene_patch_apply");
    let temp_home = temp_root.join(".gravimera");

    let child = Command::new(bin)
        .current_dir(&temp_root)
        .env("GRAVIMERA_HOME", &temp_home)
        .args([
            "--headless",
            "--headless-seconds",
            "0",
            "--automation",
            "--automation-bind",
            &format!("127.0.0.1:{port}"),
            "--automation-disable-local-input",
            "--automation-pause-on-start",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn gravimera");
    let mut child = ChildGuard { child };

    wait_for_health(addr);

    let scorecard = serde_json::json!({
        "format_version": 1,
        "hard_gates": [
            { "kind": "schema" },
            { "kind": "budget", "max_instances": 10000, "max_portals": 10000 }
        ]
    });

    // Case A: patch a layer file and ensure only that layer changes.
    let fixture_src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scene_generation/fixtures/layers_regen/src");
    let temp_src = temp_root.join("layers_src");
    copy_dir_recursive(&fixture_src, &temp_src);
    import_sources(addr, &temp_src);
    compile_layers(addr);
    let sig1 = get_signature(addr);

    let new_layer_a_doc = serde_json::json!({
        "kind": "explicit_instances",
        "instances": [
            {
                "local_id": "a1",
                "prefab_id": "a114d7d4-d27b-5d79-926e-c435f181e1df",
                "transform": {
                    "translation": { "x": 10.0, "y": 0.0, "z": 0.0 },
                    "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                    "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
                }
            }
        ]
    });

    let patch = serde_json::json!({
        "format_version": 1,
        "request_id": "req_layer_1",
        "ops": [
            { "kind": "upsert_layer", "layer_id": "layer_a", "doc": new_layer_a_doc }
        ]
    });

    let dry = patch_validate(addr, scorecard.clone(), patch.clone());
    assert!(dry["ok"].as_bool().unwrap_or(false));
    assert!(dry["validation_report"]["hard_gates_passed"]
        .as_bool()
        .unwrap_or(false));
    assert!(contains_path(
        &dry["patch_summary"]["changed_paths"],
        "layers/layer_a.json"
    ));

    let applied = patch_apply(addr, scorecard.clone(), patch.clone());
    assert!(applied["ok"].as_bool().unwrap_or(false));
    assert!(applied["applied"].as_bool().unwrap_or(false));
    assert!(contains_path(
        &applied["patch_summary"]["changed_paths"],
        "layers/layer_a.json"
    ));

    let sig2 = get_signature(addr);
    assert_eq!(sig1["pinned_sig"], sig2["pinned_sig"]);
    assert_eq!(sig1["layer_sigs"]["layer_b"], sig2["layer_sigs"]["layer_b"]);
    assert_ne!(sig1["layer_sigs"]["layer_a"], sig2["layer_sigs"]["layer_a"]);
    assert_ne!(sig1["overall_sig"], sig2["overall_sig"]);

    // Idempotent: applying the same patch again should report no changed paths and not change the signature.
    let applied2 = patch_apply(addr, scorecard.clone(), patch);
    assert!(applied2["ok"].as_bool().unwrap_or(false));
    assert!(applied2["applied"].as_bool().unwrap_or(false));
    assert!(
        applied2["patch_summary"]["changed_paths"]
            .as_array()
            .is_some_and(|v| v.is_empty()),
        "expected empty changed_paths, got {}",
        applied2["patch_summary"]["changed_paths"]
    );
    let sig3 = get_signature(addr);
    assert_eq!(sig2["overall_sig"], sig3["overall_sig"]);

    // Case B: create a new pinned instance using deterministic local_ref derived ids.
    let minimal_fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scene_generation/fixtures/minimal/src");
    let temp_minimal = temp_root.join("minimal_src");
    copy_dir_recursive(&minimal_fixture, &temp_minimal);
    import_sources(addr, &temp_minimal);

    let patch = serde_json::json!({
        "format_version": 1,
        "request_id": "req_pinned_1",
        "ops": [
            {
                "kind": "upsert_pinned_instance",
                "local_ref": "new1",
                "prefab_id": "a114d7d4-d27b-5d79-926e-c435f181e1df",
                "transform": {
                    "translation": { "x": 2.0, "y": 0.0, "z": 0.0 },
                    "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                    "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
                }
            }
        ]
    });

    let applied = patch_apply(addr, scorecard, patch);
    assert!(applied["ok"].as_bool().unwrap_or(false));
    assert!(applied["applied"].as_bool().unwrap_or(false));

    let derived_id = applied["patch_summary"]["derived_instance_ids"]["new1"]
        .as_str()
        .expect("derived id for local_ref new1");

    let expected_key =
        format!("gravimera/scene_sources_patch/v1/scene/minimal/request/req_pinned_1/local/new1");
    let expected_uuid = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, expected_key.as_bytes());
    assert_eq!(derived_id, expected_uuid.to_string());

    let new_file = temp_minimal
        .join("pinned_instances")
        .join(format!("{derived_id}.json"));
    assert!(
        new_file.exists(),
        "expected new pinned instance file: {new_file:?}"
    );

    let sig = get_signature(addr);
    assert_eq!(sig["pinned_instances"].as_u64().unwrap_or(0), 2);

    let (status, _body) =
        http_request(addr, "POST", "/v1/shutdown", Some("{}")).expect("shutdown request");
    assert_eq!(status, 200);

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.child.try_wait().expect("try_wait") {
            assert!(status.success(), "gravimera exited with {status:?}");
            break;
        }
        if Instant::now() >= deadline {
            panic!("gravimera did not exit after /v1/shutdown");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
