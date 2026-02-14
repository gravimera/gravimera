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

fn get_signature(addr: SocketAddr) -> serde_json::Value {
    let (status, body) =
        http_request(addr, "GET", "/v1/scene_sources/signature", None).expect("signature");
    assert_eq!(status, 200, "signature status={status} body={body}");
    serde_json::from_str(&body).expect("signature json")
}

#[test]
fn procedural_layer_kinds_compile_deterministically_and_regen_is_scoped() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(v) => v,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("Skipping scene layer kinds test: bind not permitted ({err}).");
            return;
        }
        Err(err) => panic!("bind ephemeral port: {err}"),
    };
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);

    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();

    let bin = env!("CARGO_BIN_EXE_gravimera");
    let temp_root = make_temp_dir("gravimera_scene_layer_kinds_compile");
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

    let fixture_src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scene_generation/fixtures/procedural_layers_v1/src");
    assert!(fixture_src.join("index.json").exists());

    let temp_src = temp_root.join("scene_src");
    copy_dir_recursive(&fixture_src, &temp_src);

    // Import (pinned only).
    let import_body = serde_json::json!({
        "src_dir": temp_src.display().to_string(),
    })
    .to_string();
    let (status, body) =
        http_request(addr, "POST", "/v1/scene_sources/import", Some(&import_body))
            .expect("import request");
    assert_eq!(status, 200, "import failed: status={status} body={body}");

    // Compile all layers twice: signatures must match.
    let (status, body) =
        http_request(addr, "POST", "/v1/scene_sources/compile", Some("{}")).expect("compile 1");
    assert_eq!(status, 200, "compile failed: status={status} body={body}");
    let sig1 = get_signature(addr);

    let (status, body) =
        http_request(addr, "POST", "/v1/scene_sources/compile", Some("{}")).expect("compile 2");
    assert_eq!(status, 200, "compile failed: status={status} body={body}");
    let sig2 = get_signature(addr);

    assert_eq!(sig1["overall_sig"], sig2["overall_sig"]);
    assert_eq!(sig1["pinned_sig"], sig2["pinned_sig"]);
    assert_eq!(sig1["layer_sigs"], sig2["layer_sigs"]);
    assert_eq!(sig1["total_instances"], sig2["total_instances"]);

    // Edit one layer on disk, reload sources, then regenerate only that layer.
    let grid_path = temp_src.join("layers/grid_a.json");
    let updated_grid = r#"{
  "count": {
    "x": 1,
    "z": 3
  },
  "format_version": 1,
  "kind": "grid_instances",
  "layer_id": "grid_a",
  "origin": {
    "x": 0.0,
    "y": 0.0,
    "z": 0.0
  },
  "prefab_id": "a114d7d4-d27b-5d79-926e-c435f181e1df",
  "step": {
    "x": 1.0,
    "z": 2.0
  }
}
"#;
    std::fs::write(&grid_path, updated_grid).expect("write updated grid");

    let (status, body) =
        http_request(addr, "POST", "/v1/scene_sources/reload", Some("{}")).expect("reload");
    assert_eq!(status, 200, "reload failed: status={status} body={body}");

    let regen_body = serde_json::json!({ "layer_id": "grid_a" }).to_string();
    let (status, body) = http_request(
        addr,
        "POST",
        "/v1/scene_sources/regenerate_layer",
        Some(&regen_body),
    )
    .expect("regen grid");
    assert_eq!(status, 200, "regen failed: status={status} body={body}");
    let regen_json: serde_json::Value = serde_json::from_str(&body).expect("regen json");
    assert_eq!(regen_json["despawned"].as_u64().unwrap_or(0), 3);

    let sig_after_grid = get_signature(addr);
    assert_eq!(sig1["pinned_sig"], sig_after_grid["pinned_sig"]);
    assert_eq!(
        sig1["layer_sigs"]["path_a"],
        sig_after_grid["layer_sigs"]["path_a"]
    );
    assert_ne!(
        sig1["layer_sigs"]["grid_a"],
        sig_after_grid["layer_sigs"]["grid_a"]
    );

    // Now edit the polyline layer and regenerate only that layer.
    let path_path = temp_src.join("layers/path_a.json");
    let updated_path = r#"{
  "format_version": 1,
  "kind": "polyline_instances",
  "layer_id": "path_a",
  "points": [
    {
      "x": 0.0,
      "y": 0.0,
      "z": 0.0
    },
    {
      "x": 2.0,
      "y": 0.0,
      "z": 0.0
    },
    {
      "x": 2.0,
      "y": 0.0,
      "z": 3.0
    }
  ],
  "prefab_id": "6b207454-d89d-5230-b2e1-c447a96cb3fb",
  "spacing": 1.0,
  "start_offset": 0.0
}
"#;
    std::fs::write(&path_path, updated_path).expect("write updated path");

    let (status, body) =
        http_request(addr, "POST", "/v1/scene_sources/reload", Some("{}")).expect("reload 2");
    assert_eq!(status, 200, "reload failed: status={status} body={body}");

    let regen_body = serde_json::json!({ "layer_id": "path_a" }).to_string();
    let (status, body) = http_request(
        addr,
        "POST",
        "/v1/scene_sources/regenerate_layer",
        Some(&regen_body),
    )
    .expect("regen path");
    assert_eq!(status, 200, "regen failed: status={status} body={body}");
    let regen_json: serde_json::Value = serde_json::from_str(&body).expect("regen json");
    assert_eq!(regen_json["spawned"].as_u64().unwrap_or(0), 1);
    assert_eq!(regen_json["despawned"].as_u64().unwrap_or(0), 0);

    let sig_after_path = get_signature(addr);

    // Pinned content and other layers must remain unchanged.
    assert_eq!(sig1["pinned_sig"], sig_after_path["pinned_sig"]);
    assert_eq!(
        sig_after_grid["layer_sigs"]["grid_a"],
        sig_after_path["layer_sigs"]["grid_a"]
    );

    // The regenerated layer must change.
    assert_ne!(
        sig_after_grid["layer_sigs"]["path_a"],
        sig_after_path["layer_sigs"]["path_a"]
    );

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

