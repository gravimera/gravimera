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

fn collect_relative_json_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(next) = stack.pop() {
        let entries = std::fs::read_dir(&next).expect("read_dir");
        for entry in entries {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|v| v.to_str()) == Some("json") {
                let rel = path.strip_prefix(root).expect("strip_prefix");
                out.push(rel.to_path_buf());
            }
        }
    }
    out.sort();
    out
}

fn read_file_bytes(path: &Path) -> Vec<u8> {
    std::fs::read(path).expect("read file")
}

fn diff_bytes(expected: &[u8], got: &[u8]) -> String {
    let expected = String::from_utf8_lossy(expected);
    let got = String::from_utf8_lossy(got);
    let mut out = String::new();
    for (i, (a, b)) in expected.lines().zip(got.lines()).enumerate() {
        if a != b {
            out.push_str(&format!("line {}:\n- {}\n+ {}\n", i + 1, a, b));
        }
    }
    if out.is_empty() && expected != got {
        out.push_str("files differ (binary)\n");
    }
    out
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

#[test]
fn scene_sources_roundtrip_minimal_fixture_via_automation() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(v) => v,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("Skipping scene sources roundtrip test: bind not permitted ({err}).");
            return;
        }
        Err(err) => panic!("bind ephemeral port: {err}"),
    };
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);

    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();

    let bin = env!("CARGO_BIN_EXE_gravimera");
    let temp_root = make_temp_dir("gravimera_scene_sources_roundtrip");
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

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match http_request(addr, "GET", "/v1/health", None) {
            Ok((200, body)) if body.contains("\"ok\":true") => break,
            Ok(_) => {}
            Err(_) => {}
        }
        if Instant::now() >= deadline {
            panic!("Automation API did not become ready in time");
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let fixture_src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scene_generation/fixtures/minimal/src");
    assert!(fixture_src.join("index.json").exists());

    let out_src = temp_root.join("exported_src");

    let import_body = serde_json::json!({
        "src_dir": fixture_src.display().to_string(),
    })
    .to_string();
    let (status, body) =
        http_request(addr, "POST", "/v1/scene_sources/import", Some(&import_body))
            .expect("import request");
    assert_eq!(status, 200, "import failed: status={status} body={body}");
    assert!(body.contains("\"ok\":true"), "unexpected import body: {body}");

    let export_body = serde_json::json!({
        "out_dir": out_src.display().to_string(),
    })
    .to_string();
    let (status, body) =
        http_request(addr, "POST", "/v1/scene_sources/export", Some(&export_body))
            .expect("export request");
    assert_eq!(status, 200, "export failed: status={status} body={body}");
    assert!(body.contains("\"ok\":true"), "unexpected export body: {body}");

    let fixture_files = collect_relative_json_files(&fixture_src);
    let out_files = collect_relative_json_files(&out_src);
    assert_eq!(fixture_files, out_files, "exported file set mismatch");

    for rel in fixture_files {
        let expected_path = fixture_src.join(&rel);
        let got_path = out_src.join(&rel);
        let expected = read_file_bytes(&expected_path);
        let got = read_file_bytes(&got_path);
        assert_eq!(
            expected,
            got,
            "file mismatch for {} diff:\n{}",
            rel.display(),
            diff_bytes(&expected, &got)
        );
    }

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

