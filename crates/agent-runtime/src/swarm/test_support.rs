use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use solaris_core_ir::{
    AgentRole, AspectPreset, BrandPlaybook, CommandMeta, ExportJobStatus, ExportRequest,
    MediaAsset, OpResult, Project, ProjectEngine, SavedCodeAsset, TimelineCommand, TranscriptWord,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;

use super::SwarmRunRequest;
use super::request::SwarmRunMode;
use crate::backend::{BoxFuture, CapturedFrame, KeyframeResult, SolarisBackend};
use crate::error::AgentError;
use crate::openrouter::{OpenRouterClient, OpenRouterConfig};

pub(crate) struct InMemBackend {
    pub(crate) engine: RwLock<ProjectEngine>,
    pub(crate) assets: Vec<SavedCodeAsset>,
    pub(crate) brand: BrandPlaybook,
    pub(crate) transcribed: Mutex<Vec<String>>,
}

impl InMemBackend {
    pub(crate) fn new(project: Project, assets: Vec<SavedCodeAsset>) -> Arc<Self> {
        Arc::new(Self {
            engine: RwLock::new(ProjectEngine::new(project)),
            assets,
            brand: BrandPlaybook::solaris_default(),
            transcribed: Mutex::new(Vec::new()),
        })
    }
}

impl SolarisBackend for InMemBackend {
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
        Box::pin(async { Ok(self.engine.read().await.project().clone()) })
    }
    fn apply_op(
        &self,
        _id: &str,
        cmd: TimelineCommand,
        meta: CommandMeta,
    ) -> BoxFuture<'_, Result<(OpResult, Project), AgentError>> {
        Box::pin(async move {
            let mut g = self.engine.write().await;
            let (res, _) = g
                .apply(cmd, meta)
                .map_err(|e| AgentError::ToolError(e.to_string()))?;
            Ok((res, g.project().clone()))
        })
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
        tid: &str,
        aid: &str,
        role: AgentRole,
    ) -> BoxFuture<'_, Result<(), AgentError>> {
        let tid = tid.to_string();
        let aid = aid.to_string();
        Box::pin(async move {
            let mut g = self.engine.write().await;
            g.acquire_track_lock(&tid, aid, role)
                .map(|_| ())
                .map_err(|e| AgentError::ToolError(e.to_string()))
        })
    }
    fn release_track_lock(
        &self,
        _pid: &str,
        tid: &str,
        aid: &str,
    ) -> BoxFuture<'_, Result<(), AgentError>> {
        let tid = tid.to_string();
        let aid = aid.to_string();
        Box::pin(async move {
            let mut g = self.engine.write().await;
            g.release_track_lock(&tid, &aid)
                .map(|_| ())
                .map_err(|e| AgentError::ToolError(e.to_string()))
        })
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
        mid: &str,
        _lang: Option<String>,
    ) -> BoxFuture<'_, Result<Vec<TranscriptWord>, AgentError>> {
        let mid = mid.to_string();
        Box::pin(async move {
            self.transcribed
                .lock()
                .map_err(|_| AgentError::Backend("transcript log poisoned".into()))?
                .push(mid);
            Ok(vec![])
        })
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
        Box::pin(async { Ok(vec![self.brand.clone()]) })
    }
    fn get_brand(&self, _bid: &str) -> BoxFuture<'_, Result<BrandPlaybook, AgentError>> {
        Box::pin(async { Ok(self.brand.clone()) })
    }
    fn list_assets(&self) -> BoxFuture<'_, Result<Vec<SavedCodeAsset>, AgentError>> {
        Box::pin(async { Ok(self.assets.clone()) })
    }
    fn save_asset(&self, a: SavedCodeAsset) -> BoxFuture<'_, Result<SavedCodeAsset, AgentError>> {
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

pub(crate) fn offline_client() -> OpenRouterClient {
    OpenRouterClient::new(OpenRouterConfig::default())
}

pub(crate) fn stub_client(base_url: String) -> OpenRouterClient {
    OpenRouterClient::new(OpenRouterConfig {
        api_key: Some("test-key".to_string()),
        base_url,
        ..OpenRouterConfig::default()
    })
}

pub(crate) fn local_request(prompt: &str) -> SwarmRunRequest {
    SwarmRunRequest {
        prompt: prompt.to_string(),
        model_override: None,
        allow_local_fallback: true,
        max_turns_per_worker: Some(2),
        skip_qa: true,
        approved_plan: None,
        mode: SwarmRunMode::Auto,
        selected_clip_ids: Vec::new(),
        playhead_sec: None,
        conversation_history: Vec::new(),
    }
}

pub(crate) enum StubReply {
    Json(String),
    Stall,
}

pub(crate) fn chat_reply(content: &str) -> StubReply {
    StubReply::Json(
        json!({
            "id": "stub",
            "model": "stub/model",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": content },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
        })
        .to_string(),
    )
}

/// Serves `POST` chat requests from `replies` in arrival order (the last reply repeats).
/// `GET` catalog requests get an empty model list and never consume a reply.
pub(crate) async fn spawn_openrouter_stub(replies: Vec<StubReply>) -> String {
    assert!(!replies.is_empty(), "stub needs at least one reply");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind openrouter stub");
    let addr = listener.local_addr().expect("stub address");
    let replies = Arc::new(replies);
    let chat_calls = Arc::new(AtomicUsize::new(0));
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let replies = Arc::clone(&replies);
            let chat_calls = Arc::clone(&chat_calls);
            tokio::spawn(async move { serve_connection(socket, &replies, &chat_calls).await });
        }
    });
    format!("http://{addr}")
}

async fn serve_connection(mut socket: TcpStream, replies: &[StubReply], chat_calls: &AtomicUsize) {
    let request = read_request(&mut socket).await;
    if request.starts_with("GET ") {
        write_response(&mut socket, r#"{"data":[]}"#).await;
        return;
    }
    let index = chat_calls
        .fetch_add(1, Ordering::SeqCst)
        .min(replies.len() - 1);
    match &replies[index] {
        StubReply::Json(body) => write_response(&mut socket, body).await,
        StubReply::Stall => std::future::pending::<()>().await,
    }
}

async fn read_request(socket: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = match socket.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        buf.extend_from_slice(&chunk[..read]);
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
            let body_len = head
                .lines()
                .find_map(|line| {
                    line.strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            if buf.len() >= end + 4 + body_len {
                break;
            }
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

async fn write_response(socket: &mut TcpStream, body: &str) {
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = socket.write_all(head.as_bytes()).await;
    let _ = socket.write_all(body.as_bytes()).await;
    let _ = socket.shutdown().await;
}
