use bevy::log::{debug, warn};
use std::process::Command;
use std::sync::OnceLock;

#[derive(Clone, Debug)]
pub(crate) struct SystemProxySettings {
    pub(crate) http_proxy: Option<String>,
    pub(crate) https_proxy: Option<String>,
    pub(crate) all_proxy: Option<String>,
    pub(crate) no_proxy: Option<String>,
    pub(crate) source: &'static str,
    pub(crate) notes: Vec<String>,
}

impl SystemProxySettings {
    fn is_empty(&self) -> bool {
        self.http_proxy.is_none()
            && self.https_proxy.is_none()
            && self.all_proxy.is_none()
            && self.no_proxy.is_none()
    }
}

pub(crate) fn apply_system_proxy_to_curl_command(cmd: &mut Command, target_url: &str) {
    if system_proxy_disabled() {
        return;
    }
    if env_has_any_proxy() {
        return;
    }

    static CACHED: OnceLock<Option<SystemProxySettings>> = OnceLock::new();
    let Some(settings) = CACHED.get_or_init(detect_system_proxy).as_ref() else {
        return;
    };
    if settings.is_empty() {
        return;
    }

    if let Some(value) = &settings.http_proxy {
        cmd.env("http_proxy", value);
        cmd.env("HTTP_PROXY", value);
    }
    if let Some(value) = &settings.https_proxy {
        cmd.env("https_proxy", value);
        cmd.env("HTTPS_PROXY", value);
    }
    if let Some(value) = &settings.all_proxy {
        cmd.env("all_proxy", value);
        cmd.env("ALL_PROXY", value);
    }
    if let Some(value) = &settings.no_proxy {
        cmd.env("no_proxy", value);
        cmd.env("NO_PROXY", value);
    }

    debug!(
        "System proxy applied to curl (source={}, target_url={})",
        settings.source, target_url
    );
}

fn system_proxy_disabled() -> bool {
    std::env::var_os("GRAVIMERA_DISABLE_SYSTEM_PROXY").is_some_and(|v| !v.is_empty() && v != "0")
}

fn env_has_any_proxy() -> bool {
    [
        "http_proxy",
        "HTTP_PROXY",
        "https_proxy",
        "HTTPS_PROXY",
        "all_proxy",
        "ALL_PROXY",
    ]
    .into_iter()
    .any(|key| std::env::var_os(key).is_some_and(|v| !v.is_empty()))
}

fn detect_system_proxy() -> Option<SystemProxySettings> {
    #[cfg(target_os = "macos")]
    {
        if let Some(settings) = detect_macos_proxy() {
            log_detection(&settings);
            return Some(settings);
        }
        return None;
    }

    #[cfg(windows)]
    {
        if let Some(settings) = detect_windows_proxy() {
            log_detection(&settings);
            return Some(settings);
        }
        return None;
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    {
        None
    }
}

fn log_detection(settings: &SystemProxySettings) {
    if settings.is_empty() {
        debug!("System proxy detection: none (source={})", settings.source);
        return;
    }
    let http = settings.http_proxy.as_deref().map(redact_proxy_url);
    let https = settings.https_proxy.as_deref().map(redact_proxy_url);
    let all = settings.all_proxy.as_deref().map(redact_proxy_url);
    debug!(
        "System proxy detected (source={}, http_proxy={:?}, https_proxy={:?}, all_proxy={:?})",
        settings.source, http, https, all
    );
    for note in &settings.notes {
        debug!("System proxy note: {note}");
    }
}

fn redact_proxy_url(url: &str) -> String {
    let url = url.trim();
    let Some(scheme_sep) = url.find("://") else {
        return url.to_string();
    };
    let (scheme, rest) = url.split_at(scheme_sep + 3);
    let Some(at) = rest.find('@') else {
        return url.to_string();
    };
    format!("{scheme}<redacted>@{}", &rest[(at + 1)..])
}

fn format_host_port(host: &str, port: u16) -> String {
    let host = host.trim();
    if host.contains(':') && !host.starts_with('[') && !host.ends_with(']') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn http_proxy_url(host: &str, port: u16) -> String {
    format!("http://{}", format_host_port(host, port))
}

fn socks5h_proxy_url(host: &str, port: u16) -> String {
    format!("socks5h://{}", format_host_port(host, port))
}

#[cfg(target_os = "macos")]
fn detect_macos_proxy() -> Option<SystemProxySettings> {
    let output = Command::new("scutil").arg("--proxy").output().ok()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!("System proxy detection failed (scutil --proxy): {stderr}");
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    Some(parse_macos_scutil_proxy(&stdout))
}

#[cfg(target_os = "macos")]
fn parse_macos_scutil_proxy(text: &str) -> SystemProxySettings {
    use std::collections::HashMap;

    let mut values: HashMap<String, String> = HashMap::new();
    let mut exceptions: Vec<String> = Vec::new();
    let mut in_exceptions = false;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if in_exceptions {
            if line.starts_with('}') {
                in_exceptions = false;
                continue;
            }
            if let Some((_, value)) = line.split_once(" : ") {
                let v = value.trim();
                if !v.is_empty() {
                    exceptions.push(v.to_string());
                }
            }
            continue;
        }

        let Some((key, value)) = line.split_once(" : ") else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if key == "ExceptionsList" {
            in_exceptions = true;
        }
        values.insert(key.to_string(), value.to_string());
    }

    let mut notes = Vec::new();
    let pac_enabled = values
        .get("ProxyAutoConfigEnable")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0)
        != 0;
    if pac_enabled {
        notes.push(
            "PAC is enabled (ProxyAutoConfigEnable=1); Gravimera does not evaluate PAC scripts."
                .into(),
        );
    }

    let http_proxy = build_macos_http_proxy(&values);
    let https_proxy = build_macos_https_proxy(&values);
    let all_proxy = if http_proxy.is_none() && https_proxy.is_none() {
        build_macos_socks_proxy(&values)
    } else {
        None
    };

    let no_proxy = {
        let mut items = Vec::new();
        for raw in exceptions {
            let item = raw.trim();
            if item.is_empty() {
                continue;
            }
            if let Some(rest) = item.strip_prefix("*.") {
                items.push(format!(".{rest}"));
                continue;
            }
            items.push(item.to_string());
        }
        if items.is_empty() {
            None
        } else {
            Some(items.join(","))
        }
    };

    SystemProxySettings {
        http_proxy,
        https_proxy,
        all_proxy,
        no_proxy,
        source: "macos:scutil",
        notes,
    }
}

#[cfg(target_os = "macos")]
fn build_macos_http_proxy(values: &std::collections::HashMap<String, String>) -> Option<String> {
    let enabled = values
        .get("HTTPEnable")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0)
        != 0;
    if !enabled {
        return None;
    }
    let host = values.get("HTTPProxy")?.trim();
    let port = values.get("HTTPPort")?.trim().parse::<u16>().ok()?;
    if host.is_empty() {
        return None;
    }
    Some(http_proxy_url(host, port))
}

#[cfg(target_os = "macos")]
fn build_macos_https_proxy(values: &std::collections::HashMap<String, String>) -> Option<String> {
    let enabled = values
        .get("HTTPSEnable")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0)
        != 0;
    if !enabled {
        return None;
    }
    let host = values.get("HTTPSProxy")?.trim();
    let port = values.get("HTTPSPort")?.trim().parse::<u16>().ok()?;
    if host.is_empty() {
        return None;
    }
    Some(http_proxy_url(host, port))
}

#[cfg(target_os = "macos")]
fn build_macos_socks_proxy(values: &std::collections::HashMap<String, String>) -> Option<String> {
    let enabled = values
        .get("SOCKSEnable")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0)
        != 0;
    if !enabled {
        return None;
    }
    let host = values.get("SOCKSProxy")?.trim();
    let port = values.get("SOCKSPort")?.trim().parse::<u16>().ok()?;
    if host.is_empty() {
        return None;
    }
    Some(socks5h_proxy_url(host, port))
}

#[cfg(windows)]
fn detect_windows_proxy() -> Option<SystemProxySettings> {
    let mut notes = Vec::new();
    let registry = detect_windows_internet_settings_proxy(&mut notes);
    if let Some(mut settings) = registry {
        settings.notes = notes;
        return Some(settings);
    }
    let winhttp = detect_windows_winhttp_proxy(&mut notes);
    if let Some(mut settings) = winhttp {
        settings.notes = notes;
        return Some(settings);
    }
    None
}

#[cfg(windows)]
fn detect_windows_internet_settings_proxy(notes: &mut Vec<String>) -> Option<SystemProxySettings> {
    let ps = r#"
$ErrorActionPreference = 'Stop'
$p = Get-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings'
$obj = [PSCustomObject]@{
  ProxyEnable   = $p.ProxyEnable
  ProxyServer   = $p.ProxyServer
  ProxyOverride = $p.ProxyOverride
  AutoConfigURL = $p.AutoConfigURL
  AutoDetect    = $p.AutoDetect
}
$obj | ConvertTo-Json -Compress
"#;

    let output = Command::new("powershell")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(ps)
        .output()
        .ok()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!("System proxy detection failed (powershell Internet Settings): {stderr}");
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = match serde_json::from_str(stdout.trim()) {
        Ok(json) => json,
        Err(err) => {
            warn!("System proxy detection failed (powershell JSON parse): {err}");
            return None;
        }
    };

    let enabled = json
        .get("ProxyEnable")
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        != 0;
    let server = json
        .get("ProxyServer")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if !enabled || server.is_empty() {
        return None;
    }

    let auto_config_url = json
        .get("AutoConfigURL")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let auto_detect = json.get("AutoDetect").and_then(|v| v.as_i64()).unwrap_or(0) != 0;
    if !auto_config_url.is_empty() || auto_detect {
        notes.push("Windows system proxy has AutoDetect/PAC configured; Gravimera uses only explicit ProxyServer settings.".into());
    }

    let override_raw = json
        .get("ProxyOverride")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let no_proxy = windows_proxy_override_to_no_proxy(override_raw);

    let (http_proxy, https_proxy, all_proxy) = parse_windows_proxy_server(server);

    Some(SystemProxySettings {
        http_proxy,
        https_proxy,
        all_proxy,
        no_proxy,
        source: "windows:internet_settings",
        notes: Vec::new(),
    })
}

#[cfg(windows)]
fn detect_windows_winhttp_proxy(notes: &mut Vec<String>) -> Option<SystemProxySettings> {
    let output = Command::new("netsh")
        .arg("winhttp")
        .arg("show")
        .arg("proxy")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    if text.to_ascii_lowercase().contains("direct access") {
        return None;
    }
    // Best-effort parse. Output format may vary by locale.
    let mut proxy_server = None::<String>;
    let mut bypass_list = None::<String>;
    for line in text.lines() {
        let line = line.trim();
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim();
            if k.contains("proxy server") && !v.is_empty() && v != "(none)" {
                proxy_server = Some(v.to_string());
            }
            if (k.contains("bypass") || k.contains("bypass list")) && !v.is_empty() && v != "(none)"
            {
                bypass_list = Some(v.to_string());
            }
        }
    }
    let Some(server) = proxy_server else {
        return None;
    };

    notes.push("Using WinHTTP proxy settings (netsh winhttp show proxy).".into());
    let (http_proxy, https_proxy, all_proxy) = parse_windows_proxy_server(&server);

    Some(SystemProxySettings {
        http_proxy,
        https_proxy,
        all_proxy,
        no_proxy: bypass_list.and_then(|v| windows_proxy_override_to_no_proxy(&v)),
        source: "windows:winhttp",
        notes: Vec::new(),
    })
}

#[cfg(windows)]
fn windows_proxy_override_to_no_proxy(value: &str) -> Option<String> {
    let mut items = Vec::new();
    for raw in value.split(';') {
        let item = raw.trim();
        if item.is_empty() {
            continue;
        }
        if let Some(rest) = item.strip_prefix("*.") {
            items.push(format!(".{rest}"));
            continue;
        }
        items.push(item.to_string());
    }
    if items.is_empty() {
        None
    } else {
        Some(items.join(","))
    }
}

#[cfg(windows)]
fn parse_windows_proxy_server(value: &str) -> (Option<String>, Option<String>, Option<String>) {
    let value = value.trim();
    if value.is_empty() {
        return (None, None, None);
    }

    if value.contains('=') {
        let mut http_proxy = None;
        let mut https_proxy = None;
        let mut all_proxy = None;
        for part in value.split(';') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let Some((proto, server)) = part.split_once('=') else {
                continue;
            };
            let proto = proto.trim().to_ascii_lowercase();
            let server = server.trim();
            if server.is_empty() {
                continue;
            }
            let url = windows_proxy_url_for_proto(&proto, server);
            match proto.as_str() {
                "http" => http_proxy = Some(url),
                "https" => https_proxy = Some(url),
                "socks" | "socks5" => all_proxy = Some(url),
                _ => {}
            }
        }
        return (http_proxy, https_proxy, all_proxy);
    }

    // Single proxy for all protocols.
    let url = if value.contains("://") {
        value.to_string()
    } else {
        format!("http://{value}")
    };
    (Some(url.clone()), Some(url), None)
}

#[cfg(windows)]
fn windows_proxy_url_for_proto(proto: &str, server: &str) -> String {
    if server.contains("://") {
        return server.to_string();
    }
    if matches!(proto, "socks" | "socks5") {
        return format!("socks5h://{server}");
    }
    // Windows ProxyServer values are typically host:port for an HTTP proxy.
    format!("http://{server}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_proxy_url_hides_userinfo() {
        assert_eq!(
            redact_proxy_url("http://user:pass@127.0.0.1:7891"),
            "http://<redacted>@127.0.0.1:7891"
        );
        assert_eq!(
            redact_proxy_url("socks5h://u@host:1"),
            "socks5h://<redacted>@host:1"
        );
        assert_eq!(
            redact_proxy_url("http://127.0.0.1:7891"),
            "http://127.0.0.1:7891"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parse_macos_scutil_proxy_extracts_http_https_socks_and_exceptions() {
        let input = r#"
<dictionary> {
  ExceptionsList : <array> {
    0 : 127.0.0.1
    1 : localhost
    2 : *.local
  }
  HTTPEnable : 1
  HTTPPort : 7891
  HTTPProxy : 127.0.0.1
  HTTPSEnable : 1
  HTTPSPort : 7891
  HTTPSProxy : 127.0.0.1
  SOCKSEnable : 1
  SOCKSPort : 7891
  SOCKSProxy : 127.0.0.1
}
"#;
        let parsed = parse_macos_scutil_proxy(input);
        assert_eq!(parsed.http_proxy.as_deref(), Some("http://127.0.0.1:7891"));
        assert_eq!(parsed.https_proxy.as_deref(), Some("http://127.0.0.1:7891"));
        assert!(parsed.all_proxy.is_none());
        assert_eq!(
            parsed.no_proxy.as_deref(),
            Some("127.0.0.1,localhost,.local")
        );
    }

    #[cfg(windows)]
    #[test]
    fn parse_windows_proxy_server_single_value_sets_http_and_https() {
        let (http, https, all) = parse_windows_proxy_server("127.0.0.1:7891");
        assert_eq!(http.as_deref(), Some("http://127.0.0.1:7891"));
        assert_eq!(https.as_deref(), Some("http://127.0.0.1:7891"));
        assert!(all.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn parse_windows_proxy_server_per_scheme_parses_parts() {
        let (http, https, all) =
            parse_windows_proxy_server("http=127.0.0.1:1;https=127.0.0.1:2;socks=127.0.0.1:3");
        assert_eq!(http.as_deref(), Some("http://127.0.0.1:1"));
        assert_eq!(https.as_deref(), Some("http://127.0.0.1:2"));
        assert_eq!(all.as_deref(), Some("socks5h://127.0.0.1:3"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_proxy_override_to_no_proxy_converts_separators() {
        assert_eq!(
            windows_proxy_override_to_no_proxy("<local>;*.local;127.0.0.1"),
            Some("<local>,.local,127.0.0.1".into())
        );
    }
}
