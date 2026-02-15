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

fn validate_sources(addr: SocketAddr, scorecard: serde_json::Value) -> serde_json::Value {
    let body = scorecard.to_string();
    let (status, body) =
        http_request(addr, "POST", "/v1/scene_sources/validate", Some(&body)).expect("validate");
    assert_eq!(status, 200, "validate failed: status={status} body={body}");
    serde_json::from_str(&body).expect("validate json")
}

fn find_violation<'a>(report: &'a serde_json::Value, code: &str) -> Option<&'a serde_json::Value> {
    report
        .get("violations")
        .and_then(|v| v.as_array())
        .and_then(|items| {
            items.iter().find(|violation| {
                violation
                    .get("code")
                    .and_then(|c| c.as_str())
                    .is_some_and(|c| c == code)
            })
        })
}

#[test]
fn validation_reports_contain_stable_codes_and_evidence() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(v) => v,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("Skipping validation scorecard test: bind not permitted ({err}).");
            return;
        }
        Err(err) => panic!("bind ephemeral port: {err}"),
    };
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);

    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();

    let bin = env!("CARGO_BIN_EXE_gravimera");
    let temp_root = make_temp_dir("gravimera_scene_validation");
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
            { "kind": "budget", "max_instances": 1000, "max_portals": 1000 }
        ]
    });

    // Case 1: Unknown prefab id referenced by sources should produce a stable violation + evidence.
    let bad_prefab_fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scene_generation/fixtures/bad_prefab/src");
    assert!(bad_prefab_fixture.join("index.json").exists());
    let temp_bad_prefab = temp_root.join("bad_prefab_src");
    copy_dir_recursive(&bad_prefab_fixture, &temp_bad_prefab);

    import_sources(addr, &temp_bad_prefab);
    let resp = validate_sources(addr, scorecard.clone());
    assert!(resp["ok"].as_bool().unwrap_or(false));
    let report = &resp["report"];
    assert!(!report["hard_gates_passed"].as_bool().unwrap_or(true));
    let violation = find_violation(report, "unknown_prefab_id").expect("unknown_prefab_id");
    assert_eq!(
        violation["evidence"]["source_path"].as_str().unwrap_or(""),
        "pinned_instances/7df705fe-80cb-4d77-9c0a-8e1472ac2dc5.json"
    );
    assert_eq!(
        violation["evidence"]["prefab_id"].as_str().unwrap_or(""),
        "7b04fd6a-ead1-4f6c-9ad2-2c0c1d68d5b2"
    );

    // Case 2: Budget exceeded should yield a stable budget code.
    let minimal_fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scene_generation/fixtures/minimal/src");
    assert!(minimal_fixture.join("index.json").exists());
    let temp_minimal = temp_root.join("minimal_src");
    copy_dir_recursive(&minimal_fixture, &temp_minimal);

    import_sources(addr, &temp_minimal);
    let resp = validate_sources(
        addr,
        serde_json::json!({
            "format_version": 1,
            "hard_gates": [
                { "kind": "schema" },
                { "kind": "budget", "max_instances": 0 }
            ]
        }),
    );
    assert!(resp["ok"].as_bool().unwrap_or(false));
    let report = &resp["report"];
    assert!(!report["hard_gates_passed"].as_bool().unwrap_or(true));
    find_violation(report, "budget_max_instances_exceeded").expect("budget_max_instances_exceeded");

    // Case 3: Unknown portal destination scene should be detected when the workspace is under a
    // realm-style `scenes/<scene_id>/src` layout.
    let portal_fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scene_generation/fixtures/portal_unknown_destination");
    let temp_portal_root = temp_root.join("portal_fixture");
    copy_dir_recursive(&portal_fixture_root, &temp_portal_root);

    let hub_src = temp_portal_root.join("scenes/hub/src");
    assert!(hub_src.join("index.json").exists());
    import_sources(addr, &hub_src);

    let resp = validate_sources(addr, scorecard);
    assert!(resp["ok"].as_bool().unwrap_or(false));
    let report = &resp["report"];
    assert!(!report["hard_gates_passed"].as_bool().unwrap_or(true));
    let violation = find_violation(report, "unknown_portal_destination_scene")
        .expect("unknown_portal_destination_scene");
    assert_eq!(
        violation["evidence"]["source_path"].as_str().unwrap_or(""),
        "portals/to_missing.json"
    );
    assert_eq!(
        violation["evidence"]["destination_scene_id"]
            .as_str()
            .unwrap_or(""),
        "missing_scene"
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
