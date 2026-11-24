use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinSet;

use crate::protocol::McpServer;

pub async fn run_stdio(server: McpServer) -> std::io::Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = Arc::new(Mutex::new(tokio::io::stdout()));
    let mut lines = BufReader::new(stdin).lines();

    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(resp) = server.handle_json_str(trimmed).await
            && let Ok(out) = serde_json::to_string(&resp)
        {
            write_single_json_line(&stdout, &out).await?;
        }
    }

    Ok(())
}

/// Forwards stdio JSON-RPC messages directly to `solaris-server`'s `/mcp`
/// Streamable HTTP endpoint concurrently so external CLI agents (Codex, Claude Code,
/// Antigravity) get full feature parity including `preview.capture_frame`,
/// `preview.seek`, `media.extract_keyframes`, and live UI cursor pulses.
pub async fn run_stdio_http_bridge(
    server_url: &str,
    auth_token: Option<&str>,
) -> std::io::Result<()> {
    let endpoint = Arc::new(format!("{}/mcp", server_url.trim_end_matches('/')));
    let auth_token = auth_token.map(str::to_string);
    let http = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap_or_else(|_| Client::new());
    let session_id: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));
    let stdout = Arc::new(Mutex::new(tokio::io::stdout()));
    let mut inflight = JoinSet::new();

    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();

    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let parsed: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                let err_resp = json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": { "code": -32700, "message": format!("Parse error: {e}") }
                });
                write_single_json_line(&stdout, &err_resp.to_string()).await?;
                continue;
            }
        };
        let req_id = parsed.get("id").cloned();
        let is_init = parsed
            .get("method")
            .and_then(|v| v.as_str())
            .is_some_and(|m| m == "initialize");

        let fut = forward_one_message(
            http.clone(),
            endpoint.clone(),
            auth_token.clone(),
            session_id.clone(),
            stdout.clone(),
            trimmed.to_string(),
            req_id,
        );

        // Reap finished background tasks continuously so long sessions never accumulate JoinSet handles.
        while inflight.try_join_next().is_some() {}

        // Run `initialize` synchronously so the negotiated `mcp-session-id` is stored before concurrent calls.
        if is_init {
            fut.await?;
        } else {
            inflight.spawn(async move {
                let _ = fut.await;
            });
        }
    }

    while inflight.join_next().await.is_some() {}
    Ok(())
}

async fn forward_one_message(
    http: Client,
    endpoint: Arc<String>,
    auth_token: Option<String>,
    session_id: Arc<RwLock<Option<String>>>,
    stdout: Arc<Mutex<tokio::io::Stdout>>,
    payload: String,
    req_id: Option<Value>,
) -> std::io::Result<()> {
    let mut req = http
        .post(endpoint.as_str())
        .header(CONTENT_TYPE, "application/json")
        .body(payload);
    if let Some(tok) = &auth_token {
        req = req.header(AUTHORIZATION, format!("Bearer {tok}"));
    }
    if let Some(sid) = session_id.read().await.clone() {
        req = req.header("mcp-session-id", sid);
    }

    match req.send().await {
        Ok(resp) => {
            if let Some(sid) = resp
                .headers()
                .get("mcp-session-id")
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                *session_id.write().await = Some(sid.to_string());
            }
            let status = resp.status();
            if status == reqwest::StatusCode::NOT_FOUND {
                *session_id.write().await = None;
            }
            if status == reqwest::StatusCode::ACCEPTED {
                return Ok(());
            }
            let body = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                if let Some(id) = req_id {
                    let err_resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32603, "message": format!("solaris-server HTTP {status}: {body}") }
                    });
                    write_single_json_line(&stdout, &err_resp.to_string()).await?;
                }
                return Ok(());
            }
            if !body.trim().is_empty() {
                write_single_json_line(&stdout, body.trim()).await?;
            }
        }
        Err(err) => {
            if let Some(id) = req_id {
                let err_resp = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32603, "message": format!("Failed to reach solaris-server at {endpoint}: {err}") }
                });
                write_single_json_line(&stdout, &err_resp.to_string()).await?;
            }
        }
    }
    Ok(())
}

/// Ensures every emitted MCP stdio message is strictly a single line (no embedded CR/LF).
fn compact_single_line_json(raw: &str) -> String {
    if let Ok(val) = serde_json::from_str::<Value>(raw)
        && let Ok(compact) = serde_json::to_string(&val)
    {
        return compact;
    }
    raw.replace(['\r', '\n'], " ")
}

async fn write_single_json_line(
    stdout: &Mutex<tokio::io::Stdout>,
    line: &str,
) -> std::io::Result<()> {
    let single = compact_single_line_json(line);
    let mut guard = stdout.lock().await;
    guard.write_all(single.as_bytes()).await?;
    guard.write_all(b"\n").await?;
    guard.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compacts_multiline_json_to_single_stdio_line() {
        let pretty = "{\n  \"jsonrpc\": \"2.0\",\n  \"id\": 1,\n  \"result\": {}\n}";
        let line = compact_single_line_json(pretty);
        assert!(!line.contains('\n'));
        assert!(!line.contains('\r'));
        let parsed: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["id"], 1);
    }
}
