use std::collections::HashMap;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::protocol::{DEFAULT_PROTOCOL_VERSION, McpServer};

const MAX_MCP_SESSIONS: usize = 256;
const SESSION_IDLE_TTL: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone)]
struct McpSession {
    protocol_version: String,
    last_seen: Instant,
}

#[derive(Default)]
pub struct McpSessionStore {
    sessions: RwLock<HashMap<String, McpSession>>,
}

impl McpSessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn create(&self, protocol_version: String) -> String {
        let now = Instant::now();
        let sid = Uuid::new_v4().to_string();
        let mut guard = self.sessions.write().await;
        guard.retain(|_, s| now.duration_since(s.last_seen) < SESSION_IDLE_TTL);
        while guard.len() >= MAX_MCP_SESSIONS {
            if let Some(oldest) = guard
                .iter()
                .min_by_key(|(_, s)| s.last_seen)
                .map(|(k, _)| k.clone())
            {
                guard.remove(&oldest);
            } else {
                break;
            }
        }
        guard.insert(
            sid.clone(),
            McpSession {
                protocol_version,
                last_seen: now,
            },
        );
        sid
    }

    pub async fn touch_and_version(&self, sid: &str) -> Option<String> {
        let now = Instant::now();
        let mut guard = self.sessions.write().await;
        let session = guard.get_mut(sid)?;
        if now.duration_since(session.last_seen) >= SESSION_IDLE_TTL {
            guard.remove(sid);
            return None;
        }
        session.last_seen = now;
        Some(session.protocol_version.clone())
    }

    pub async fn update_version(&self, sid: &str, protocol_version: String) {
        let mut guard = self.sessions.write().await;
        if let Some(session) = guard.get_mut(sid) {
            session.protocol_version = protocol_version;
            session.last_seen = Instant::now();
        }
    }

    pub async fn remove(&self, sid: &str) -> bool {
        self.sessions.write().await.remove(sid).is_some()
    }
}

pub async fn mcp_post_handler(
    server: &McpServer,
    sessions: &McpSessionStore,
    headers: &HeaderMap,
    body: String,
) -> Response {
    let incoming_sid = headers
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let parsed: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => {
            let resp = server
                .handle_json_str_with_version(&body, Some(DEFAULT_PROTOCOL_VERSION))
                .await;
            let json = resp
                .and_then(|r| serde_json::to_string(&r).ok())
                .unwrap_or_else(|| "{}".to_string());
            let mut out = Response::new(Body::from(json));
            *out.status_mut() = StatusCode::OK;
            out.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
            return out;
        }
    };

    let is_initialize = parsed
        .get("method")
        .and_then(|m| m.as_str())
        .is_some_and(|m| m == "initialize");

    let (session_id, session_version) = if is_initialize {
        if let Some(existing) = incoming_sid.as_deref()
            && let Some(ver) = sessions.touch_and_version(existing).await
        {
            (Some(existing.to_string()), ver)
        } else {
            (
                Some(sessions.create(DEFAULT_PROTOCOL_VERSION.to_string()).await),
                DEFAULT_PROTOCOL_VERSION.to_string(),
            )
        }
    } else if let Some(sid) = incoming_sid {
        let Some(ver) = sessions.touch_and_version(&sid).await else {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("MCP session expired or invalidated"))
                .unwrap();
        };
        (Some(sid), ver)
    } else {
        // Handle headerless non-initialize calls statelessly without inserting or evicting sessions.
        (None, DEFAULT_PROTOCOL_VERSION.to_string())
    };

    let Some(resp) = server
        .handle_json_rpc_with_version(parsed, Some(&session_version))
        .await
    else {
        let mut out = Response::new(Body::empty());
        *out.status_mut() = StatusCode::ACCEPTED;
        if let Some(ref sid) = session_id
            && let Ok(val) = HeaderValue::from_str(sid)
        {
            out.headers_mut().insert("mcp-session-id", val);
        }
        return out;
    };

    if is_initialize
        && let Some(ref sid) = session_id
        && let Some(negotiated) = resp
            .result
            .as_ref()
            .and_then(|r| r.get("protocolVersion"))
            .and_then(|v| v.as_str())
    {
        sessions.update_version(sid, negotiated.to_string()).await;
    }

    let json = serde_json::to_string(&resp).unwrap_or_else(|_| "{}".to_string());
    let mut out = Response::new(Body::from(json));
    *out.status_mut() = StatusCode::OK;
    out.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    if let Some(ref sid) = session_id
        && let Ok(val) = HeaderValue::from_str(sid)
    {
        out.headers_mut().insert("mcp-session-id", val);
    }
    out
}

pub async fn mcp_delete_handler(sessions: &McpSessionStore, headers: &HeaderMap) -> StatusCode {
    let Some(sid) = headers
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return StatusCode::BAD_REQUEST;
    };
    if sessions.remove(sid).await {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solaris_agent_runtime::{
        AgentError, BoxFuture, CapturedFrame, KeyframeResult, SolarisBackend,
    };
    use solaris_core_ir::{
        AgentRole, AspectPreset, BrandPlaybook, CommandMeta, ExportJobStatus, ExportRequest,
        MediaAsset, OpResult, Project, SavedCodeAsset, TimelineCommand, TranscriptWord,
    };
    use std::sync::Arc;

    struct DummyBackend;

    impl SolarisBackend for DummyBackend {
        fn list_projects(&self) -> BoxFuture<'_, Result<Vec<serde_json::Value>, AgentError>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn create_project(
            &self,
            _name: String,
            _aspect: Option<AspectPreset>,
            _brand: Option<String>,
            _prompt: Option<String>,
            _template: Option<String>,
        ) -> BoxFuture<'_, Result<Project, AgentError>> {
            Box::pin(async { Err(AgentError::Backend("unused".into())) })
        }
        fn get_project(&self, _id: &str) -> BoxFuture<'_, Result<Project, AgentError>> {
            Box::pin(async { Err(AgentError::Backend("unused".into())) })
        }
        fn apply_op(
            &self,
            _id: &str,
            _cmd: TimelineCommand,
            _meta: CommandMeta,
        ) -> BoxFuture<'_, Result<(OpResult, Project), AgentError>> {
            Box::pin(async { Err(AgentError::Backend("unused".into())) })
        }
        fn undo(&self, _id: &str) -> BoxFuture<'_, Result<Project, AgentError>> {
            Box::pin(async { Err(AgentError::Backend("unused".into())) })
        }
        fn redo(&self, _id: &str) -> BoxFuture<'_, Result<Project, AgentError>> {
            Box::pin(async { Err(AgentError::Backend("unused".into())) })
        }
        fn acquire_track_lock(
            &self,
            _pid: &str,
            _tid: &str,
            _aid: &str,
            _role: AgentRole,
        ) -> BoxFuture<'_, Result<(), AgentError>> {
            Box::pin(async { Ok(()) })
        }
        fn release_track_lock(
            &self,
            _pid: &str,
            _tid: &str,
            _aid: &str,
        ) -> BoxFuture<'_, Result<(), AgentError>> {
            Box::pin(async { Ok(()) })
        }
        fn import_media_paths(
            &self,
            _pid: &str,
            _paths: Vec<(String, Option<String>)>,
        ) -> BoxFuture<'_, Result<Vec<MediaAsset>, AgentError>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn transcribe_media(
            &self,
            _pid: &str,
            _mid: &str,
            _lang: Option<String>,
        ) -> BoxFuture<'_, Result<Vec<TranscriptWord>, AgentError>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn extract_keyframes(
            &self,
            _pid: &str,
            _mid: &str,
            _ts: Vec<f64>,
        ) -> BoxFuture<'_, Result<Vec<KeyframeResult>, AgentError>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn capture_preview_frame(
            &self,
            _pid: &str,
            _ts: f64,
        ) -> BoxFuture<'_, Result<CapturedFrame, AgentError>> {
            Box::pin(async { Err(AgentError::Backend("unused".into())) })
        }
        fn list_brands(&self) -> BoxFuture<'_, Result<Vec<BrandPlaybook>, AgentError>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn get_brand(&self, _bid: &str) -> BoxFuture<'_, Result<BrandPlaybook, AgentError>> {
            Box::pin(async { Err(AgentError::Backend("unused".into())) })
        }
        fn list_assets(&self) -> BoxFuture<'_, Result<Vec<SavedCodeAsset>, AgentError>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn save_asset(
            &self,
            a: SavedCodeAsset,
        ) -> BoxFuture<'_, Result<SavedCodeAsset, AgentError>> {
            Box::pin(async { Ok(a) })
        }
        fn start_export(
            &self,
            _pid: &str,
            _req: ExportRequest,
        ) -> BoxFuture<'_, Result<ExportJobStatus, AgentError>> {
            Box::pin(async { Err(AgentError::Backend("unused".into())) })
        }
        fn emit_cursor_pulse(
            &self,
            _pid: &str,
            _aid: &str,
            _role: AgentRole,
            _anchor: &str,
            _label: &str,
        ) -> BoxFuture<'_, Result<(), AgentError>> {
            Box::pin(async { Ok(()) })
        }
        fn emit_playhead_seek(
            &self,
            _pid: &str,
            _ts: f64,
            _aid: &str,
            _role: AgentRole,
        ) -> BoxFuture<'_, Result<(), AgentError>> {
            Box::pin(async { Ok(()) })
        }
    }

    #[tokio::test]
    async fn http_sessions_isolate_protocol_versions_and_headerless_calls_stay_stateless() {
        let server = McpServer::new(Arc::new(DummyBackend));
        let sessions = McpSessionStore::new();
        let empty_headers = HeaderMap::new();

        // Headerless non-initialize call must NOT create a session in McpSessionStore
        let stateless = mcp_post_handler(
            &server,
            &sessions,
            &empty_headers,
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#.to_string(),
        )
        .await;
        assert_eq!(stateless.status(), StatusCode::OK);
        assert!(stateless.headers().get("mcp-session-id").is_none());
        assert_eq!(sessions.sessions.read().await.len(), 0);

        // Initialize two sessions with different protocol versions
        let init_old = mcp_post_handler(
            &server,
            &sessions,
            &empty_headers,
            r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#
                .to_string(),
        )
        .await;
        let sid_old = init_old
            .headers()
            .get("mcp-session-id")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let init_new = mcp_post_handler(
            &server,
            &sessions,
            &empty_headers,
            r#"{"jsonrpc":"2.0","id":3,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#
                .to_string(),
        )
        .await;
        let sid_new = init_new
            .headers()
            .get("mcp-session-id")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(sessions.sessions.read().await.len(), 2);

        let mut h_old = HeaderMap::new();
        h_old.insert("mcp-session-id", HeaderValue::from_str(&sid_old).unwrap());
        let call_old = mcp_post_handler(
            &server,
            &sessions,
            &h_old,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"project.list","arguments":{}}}"#
                .to_string(),
        )
        .await;
        let body_old = axum::body::to_bytes(call_old.into_body(), usize::MAX)
            .await
            .unwrap();
        let val_old: serde_json::Value = serde_json::from_slice(&body_old).unwrap();
        assert!(val_old["result"].get("structuredContent").is_none());

        let mut h_new = HeaderMap::new();
        h_new.insert("mcp-session-id", HeaderValue::from_str(&sid_new).unwrap());
        let call_new = mcp_post_handler(
            &server,
            &sessions,
            &h_new,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"project.list","arguments":{}}}"#
                .to_string(),
        )
        .await;
        let body_new = axum::body::to_bytes(call_new.into_body(), usize::MAX)
            .await
            .unwrap();
        let val_new: serde_json::Value = serde_json::from_slice(&body_new).unwrap();
        assert!(val_new["result"].get("structuredContent").is_some());
    }
}
