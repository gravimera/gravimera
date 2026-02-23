use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

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

#[test]
fn automation_api_health_and_shutdown() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(v) => v,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("Skipping automation API smoke test: bind not permitted ({err}).");
            return;
        }
        Err(err) => panic!("bind ephemeral port: {err}"),
    };
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);

    let bin = env!("CARGO_BIN_EXE_gravimera");
    let temp_root = make_temp_dir("gravimera_automation_smoke");
    let temp_home = temp_root.join(".gravimera");
    let mut child = Command::new(bin)
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
            panic!("Automation API did not become ready in time");
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    // Window endpoint returns 501 in headless mode.
    let (status, _body) = http_request(addr, "GET", "/v1/window", None).expect("window");
    assert_eq!(status, 501);

    // High-level state endpoint should always work.
    let (status, body) = http_request(addr, "GET", "/v1/state", None).expect("state");
    assert_eq!(status, 200);
    assert!(body.contains("\"ok\":true"), "unexpected body: {body}");

    if let Ok(json) = serde_json::from_str::<Value>(&body) {
        let first = json
            .get("objects")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|obj| obj.get("instance_id_uuid"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        if let Some(instance_id) = first {
            let req = format!(
                "{{\"instance_ids\":[\"{instance_id}\"],\"channel\":\"ambient\"}}"
            );
            let (status, body) = http_request(
                addr,
                "POST",
                "/v1/animation/force_channel",
                Some(req.as_str()),
            )
            .expect("force animation channel");
            assert_eq!(status, 200);
            assert!(body.contains("\"ok\":true"), "unexpected body: {body}");

            let req = format!("{{\"instance_ids\":[\"{instance_id}\"],\"channel\":\"\"}}");
            let (status, body) = http_request(
                addr,
                "POST",
                "/v1/animation/force_channel",
                Some(req.as_str()),
            )
            .expect("clear forced animation channel");
            assert_eq!(status, 200);
            assert!(body.contains("\"ok\":true"), "unexpected body: {body}");
        }
    }

    // Low-level input injection endpoints are intentionally unavailable.
    for (method, path) in [
        ("GET", "/v1/input/state"),
        ("POST", "/v1/input/reset"),
        ("POST", "/v1/input/events"),
    ] {
        let body_json = if method == "POST" { Some("{}") } else { None };
        let (status, body) = http_request(addr, method, path, body_json).expect("input endpoint");
        assert_eq!(
            status, 404,
            "expected 404 for {method} {path}, got {status} body={body}"
        );
    }

    // Time stepping should remain available.
    let (status, body) = http_request(addr, "POST", "/v1/pause", Some("{}")).expect("pause");
    assert_eq!(status, 200);
    assert!(body.contains("\"ok\":true"), "unexpected body: {body}");

    let (status, body) = http_request(
        addr,
        "POST",
        "/v1/step",
        Some("{\"frames\":1,\"dt_secs\":0.0166667}"),
    )
    .expect("step 1 frame");
    assert_eq!(status, 200);
    assert!(body.contains("\"ok\":true"), "unexpected body: {body}");

    let (status, body) = http_request(addr, "POST", "/v1/resume", Some("{}")).expect("resume");
    assert_eq!(status, 200);
    assert!(body.contains("\"ok\":true"), "unexpected body: {body}");

    let (status, _body) =
        http_request(addr, "POST", "/v1/shutdown", Some("{}")).expect("shutdown request");
    assert_eq!(status, 200);

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            assert!(status.success(), "gravimera exited with {status:?}");
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("gravimera did not exit after /v1/shutdown");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
