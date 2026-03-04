use crate::intelligence::protocol::*;
use serde::{de::DeserializeOwned, Serialize};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

#[derive(Clone)]
pub(crate) struct SidecarClient {
    addr: SocketAddr,
    token: Option<String>,
    connect_timeout: Duration,
    io_timeout: Duration,
}

impl SidecarClient {
    pub(crate) fn new(addr: SocketAddr, token: Option<String>) -> Self {
        Self {
            addr,
            token,
            connect_timeout: Duration::from_millis(500),
            io_timeout: Duration::from_secs(2),
        }
    }

    pub(crate) fn health(&self) -> Result<HealthResponse, String> {
        let (status, body) = self.http_request("GET", "/v1/health", None)?;
        if status != 200 {
            return Err(format!("health failed: HTTP {status}: {body}"));
        }
        serde_json::from_str(&body).map_err(|err| format!("health decode: {err}"))
    }

    pub(crate) fn load_module(&self, module_id: &str) -> Result<LoadModuleResponse, String> {
        let req = LoadModuleRequest {
            protocol_version: PROTOCOL_VERSION,
            module_descriptor: BrainModuleDescriptor {
                module_id: module_id.to_string(),
            },
        };
        self.request_json("POST", "/v1/load_module", &req)
    }

    pub(crate) fn modules(&self) -> Result<ListModulesResponse, String> {
        let (status, body) = self.http_request("GET", "/v1/modules", None)?;
        if status != 200 {
            if let Ok(err) = serde_json::from_str::<ErrorResponse>(&body) {
                return Err(format!("HTTP {status}: {}", err.error));
            }
            return Err(format!("HTTP {status}: {body}"));
        }
        serde_json::from_str::<ListModulesResponse>(&body)
            .map_err(|err| format!("modules decode: {err}"))
    }

    pub(crate) fn spawn(
        &self,
        req: SpawnBrainInstanceRequest,
    ) -> Result<SpawnBrainInstanceResponse, String> {
        self.request_json("POST", "/v1/spawn", &req)
    }

    pub(crate) fn tick_many(&self, req: TickManyRequest) -> Result<TickManyResponse, String> {
        self.request_json("POST", "/v1/tick_many", &req)
    }

    pub(crate) fn despawn(
        &self,
        req: DespawnBrainInstanceRequest,
    ) -> Result<DespawnBrainInstanceResponse, String> {
        self.request_json("POST", "/v1/despawn", &req)
    }

    fn request_json<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        req: &Req,
    ) -> Result<Resp, String> {
        let body = serde_json::to_string(req).map_err(|err| format!("encode JSON: {err}"))?;
        let (status, raw) = self.http_request(method, path, Some(body.as_str()))?;
        if status != 200 {
            if let Ok(err) = serde_json::from_str::<ErrorResponse>(&raw) {
                return Err(format!("HTTP {status}: {}", err.error));
            }
            return Err(format!("HTTP {status}: {raw}"));
        }
        serde_json::from_str::<Resp>(&raw).map_err(|err| format!("decode JSON: {err}"))
    }

    fn http_request(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> Result<(u16, String), String> {
        let mut stream = TcpStream::connect_timeout(&self.addr, self.connect_timeout)
            .map_err(|err| format!("connect {}: {err}", self.addr))?;
        stream
            .set_read_timeout(Some(self.io_timeout))
            .map_err(|err| format!("set_read_timeout: {err}"))?;
        stream
            .set_write_timeout(Some(self.io_timeout))
            .map_err(|err| format!("set_write_timeout: {err}"))?;

        let body = body.unwrap_or("");
        let mut headers = String::new();
        headers.push_str(&format!("Host: {}\r\n", self.addr));
        headers.push_str("Connection: close\r\n");
        headers.push_str("Content-Type: application/json\r\n");
        if let Some(token) = self.token.as_deref() {
            headers.push_str(&format!("Authorization: Bearer {token}\r\n"));
        }
        headers.push_str(&format!("Content-Length: {}\r\n", body.as_bytes().len()));

        let request = format!(
            "{method} {path} HTTP/1.1\r\n\
             {headers}\r\n\
             {body}"
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|err| format!("write request: {err}"))?;
        stream.flush().ok();

        let mut raw = String::new();
        stream
            .read_to_string(&mut raw)
            .map_err(|err| format!("read response: {err}"))?;

        let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
        let status_line = head.lines().next().unwrap_or("");
        let status = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);
        Ok((status, body.to_string()))
    }
}
