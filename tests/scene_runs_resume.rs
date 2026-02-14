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

fn run_apply_patch(
    addr: SocketAddr,
    run_id: &str,
    step: u32,
    scorecard: serde_json::Value,
    patch: serde_json::Value,
) -> serde_json::Value {
    let body = serde_json::json!({
        "run_id": run_id,
        "step": step,
        "scorecard": scorecard,
        "patch": patch,
    })
    .to_string();
    let (status, body) =
        http_request(addr, "POST", "/v1/scene_sources/run_apply_patch", Some(&body)).expect("run_apply_patch");
    assert_eq!(status, 200, "run_apply_patch failed: status={status} body={body}");
    serde_json::from_str(&body).expect("run_apply_patch json")
}

fn run_status(addr: SocketAddr, run_id: &str) -> serde_json::Value {
    let body = serde_json::json!({ "run_id": run_id }).to_string();
    let (status, body) =
        http_request(addr, "POST", "/v1/scene_sources/run_status", Some(&body)).expect("run_status");
    assert_eq!(status, 200, "run_status failed: status={status} body={body}");
    serde_json::from_str(&body).expect("run_status json")
}

#[test]
fn run_directories_persist_and_steps_can_resume() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(v) => v,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("Skipping run resume test: bind not permitted ({err}).");
            return;
        }
        Err(err) => panic!("bind ephemeral port: {err}"),
    };
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);

    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();

    let bin = env!("CARGO_BIN_EXE_gravimera");
    let temp_root = make_temp_dir("gravimera_scene_runs");
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
        .join("tests/scene_generation/fixtures/minimal/src");
    let temp_src = temp_root.join("scene_src");
    copy_dir_recursive(&fixture_src, &temp_src);
    import_sources(addr, &temp_src);

    let scorecard = serde_json::json!({
        "format_version": 1,
        "hard_gates": [
            { "kind": "schema" },
            { "kind": "budget", "max_instances": 10000 }
        ]
    });

    let patch1 = serde_json::json!({
        "format_version": 1,
        "request_id": "req_run_step_1",
        "ops": [
            {
                "kind": "upsert_pinned_instance",
                "local_ref": "step1_obj",
                "prefab_id": "a114d7d4-d27b-5d79-926e-c435f181e1df",
                "transform": {
                    "translation": { "x": 5.0, "y": 0.0, "z": 0.0 },
                    "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                    "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
                }
            }
        ]
    });

    let run_id = "run_01";
    let resp1 = run_apply_patch(addr, run_id, 1, scorecard.clone(), patch1.clone());
    assert!(resp1["ok"].as_bool().unwrap_or(false));
    assert_eq!(resp1["mode"].as_str().unwrap_or(""), "executed");
    assert!(resp1["result"]["applied"].as_bool().unwrap_or(false));

    let run_dir = temp_root.join("runs").join(run_id);
    assert!(run_dir.join("run.json").exists());
    let step_dir = run_dir.join("steps/0001");
    for file in [
        "scorecard.json",
        "patch.json",
        "pre_validation_report.json",
        "apply_result.json",
        "post_signature.json",
        "complete.json",
    ] {
        assert!(step_dir.join(file).exists(), "missing {file}");
    }

    let status = run_status(addr, run_id);
    assert!(status["ok"].as_bool().unwrap_or(false));
    assert_eq!(
        status["status"]["last_complete_step"].as_u64().unwrap_or(0),
        1
    );
    assert_eq!(status["status"]["next_step"].as_u64().unwrap_or(0), 2);

    // Replay the completed step.
    let resp1b = run_apply_patch(addr, run_id, 1, scorecard.clone(), patch1);
    assert_eq!(resp1b["mode"].as_str().unwrap_or(""), "replayed");

    // Simulate a crash mid-step 2 by pre-creating the step dir without `complete.json`.
    let step2_dir = run_dir.join("steps/0002");
    std::fs::create_dir_all(&step2_dir).expect("create step2 dir");

    let patch2 = serde_json::json!({
        "format_version": 1,
        "request_id": "req_run_step_2",
        "ops": [
            {
                "kind": "upsert_pinned_instance",
                "local_ref": "step2_obj",
                "prefab_id": "a114d7d4-d27b-5d79-926e-c435f181e1df",
                "transform": {
                    "translation": { "x": 6.0, "y": 0.0, "z": 0.0 },
                    "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                    "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
                }
            }
        ]
    });

    let resp2 = run_apply_patch(addr, run_id, 2, scorecard, patch2);
    assert_eq!(resp2["mode"].as_str().unwrap_or(""), "executed");
    assert!(step2_dir.join("complete.json").exists());

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

