use serde_json::{Value, json};
use std::fs::{File, OpenOptions, create_dir_all};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

static LOGGER: OnceLock<Logger> = OnceLock::new();

pub struct Logger {
    run_id: String,
    file: Mutex<File>,
}

pub fn init() -> anyhow::Result<PathBuf> {
    let path = default_log_path();
    if let Some(parent) = path.parent() {
        create_dir_all(parent)?;
    }
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    let run_id = format!("{:016x}", rand::random::<u64>());

    let logger = Logger {
        run_id,
        file: Mutex::new(file),
    };

    let _ = LOGGER.set(logger);
    Ok(path)
}

pub fn default_log_path() -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        return home.join(".soundtest").join("soundtest.log.jsonl");
    }
    PathBuf::from(".soundtest").join("soundtest.log.jsonl")
}

pub fn info(event: &str, fields: Value) {
    write_line("info", event, fields);
}

pub fn warn(event: &str, fields: Value) {
    write_line("warn", event, fields);
}

pub fn error(event: &str, fields: Value) {
    write_line("error", event, fields);
}

pub fn event_fields() -> Value {
    json!({
        "pid": std::process::id(),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "version": env!("CARGO_PKG_VERSION"),
    })
}

fn write_line(level: &str, event: &str, fields: Value) {
    let Some(logger) = LOGGER.get() else {
        return;
    };

    let ts = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "unknown".to_owned());

    let mut obj = serde_json::Map::new();
    obj.insert("ts".to_owned(), Value::String(ts));
    obj.insert("level".to_owned(), Value::String(level.to_owned()));
    obj.insert("event".to_owned(), Value::String(event.to_owned()));
    obj.insert("run_id".to_owned(), Value::String(logger.run_id.clone()));

    match fields {
        Value::Object(map) => {
            for (k, v) in map {
                obj.insert(k, v);
            }
        }
        other => {
            obj.insert("fields".to_owned(), other);
        }
    }

    let line = Value::Object(obj).to_string();
    let mut file = match logger.file.lock() {
        Ok(f) => f,
        Err(_) => return,
    };
    let _ = writeln!(file, "{line}");
}

pub fn log_path_hint(path: &Path) -> Value {
    json!({ "log_path": path.to_string_lossy() })
}
