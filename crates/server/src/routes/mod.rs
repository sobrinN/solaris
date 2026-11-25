pub mod agent;
pub mod assets;
pub mod brands;
pub mod exports;
pub mod media;
pub mod projects;
pub mod ws;

use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Path as AxumPath, Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use solaris_agent_runtime::SolarisBackend;
use solaris_mcp_server::{McpServer, mcp_delete_handler, mcp_post_handler};
use tower_http::cors::{AllowOrigin, CorsLayer};

use solaris_media_engine::ToolchainCapabilities;

use crate::state::{AppState, constant_time_eq, validate_safe_id};

const MAX_UPLOAD_BYTES: usize = 2 * 1024 * 1024 * 1024; // 2 GiB

pub fn build_router(state: AppState) -> Router {
    let cors = build_cors_layer(&state);

    let api_protected = Router::new()
        .route(
            "/api/v1/auth/session",
            post(create_session).delete(delete_session),
        )
        .route(
            "/api/v1/projects",
            get(projects::list_projects).post(projects::create_project),
        )
        .route("/api/v1/templates", get(projects::list_templates))
        .route(
            "/api/v1/projects/{id}",
            get(projects::get_project).patch(projects::update_project_metadata),
        )
        .route("/api/v1/projects/{id}/ops", post(projects::apply_op))
        .route("/api/v1/projects/{id}/undo", post(projects::undo_project))
        .route("/api/v1/projects/{id}/redo", post(projects::redo_project))
        .route(
            "/api/v1/projects/{id}/locks/{track_id}",
            post(projects::acquire_track_lock).delete(projects::release_track_lock),
        )
        .route(
            "/api/v1/projects/{id}/media/import",
            post(media::import_media).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)),
        )
        .route(
            "/api/v1/projects/{id}/media/{media_id}",
            delete(media::delete_media),
        )
        .route("/api/v1/gfx/sandbox", post(register_gfx_sandbox))
        .route("/api/v1/projects/{id}/export", post(exports::start_export))
        .route("/api/v1/exports/{job_id}", get(exports::get_export_job))
        .route(
            "/api/v1/brands",
            get(brands::list_brands).post(brands::upsert_brand),
        )
        .route("/api/v1/brands/{id}", put(brands::update_brand))
        .route(
            "/api/v1/brands/{id}/media",
            post(brands::upload_brand_media).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)),
        )
        .route(
            "/api/v1/brands/{id}/media/{asset_id}",
            patch(brands::update_brand_media).delete(brands::delete_brand_media),
        )
        .route("/api/v1/brands/{id}/lut", get(brands::get_brand_lut))
        .route(
            "/api/v1/assets",
            get(assets::list_assets).post(assets::save_asset),
        )
        .route("/api/v1/assets/export", get(assets::export_asset_pack))
        .route("/api/v1/assets/import", post(assets::import_asset_pack))
        .route("/api/v1/agent/status", get(agent::get_agent_status))
        .route(
            "/api/v1/agent/credentials",
            post(agent::set_agent_credentials).delete(agent::clear_agent_credentials),
        )
        .route("/api/v1/agent/models", get(agent::get_recommended_models))
        .route("/api/v1/agent/skills", get(agent::get_skills))
        .route("/api/v1/agent/plan", post(agent::generate_plan))
        .route(
            "/api/v1/logs",
            get(agent::get_logs).delete(agent::clear_logs),
        )
        .route(
            "/api/v1/projects/{id}/swarm/run",
            post(agent::start_swarm_run),
        )
        .route("/api/v1/projects/{id}/qa", post(agent::run_project_qa))
        .route(
            "/api/v1/projects/{id}/swarm/{run_id}",
            get(agent::get_swarm_run),
        )
        .route(
            "/api/v1/projects/{id}/swarm/{run_id}/cancel",
            post(agent::cancel_swarm_run),
        )
        .route(
            "/api/v1/projects/{id}/media/{media_id}/transcribe",
            post(agent::transcribe_media_route),
        )
        .route("/mcp", post(mcp_http_post).delete(mcp_http_delete))
        .layer(middleware::from_fn_with_state(state.clone(), auth_guard));

    let stream_and_ws = Router::new()
        .route(
            "/api/v1/projects/{id}/media/{media_id}/stream",
            get(media::stream_media),
        )
        .route(
            "/api/v1/brands/{id}/media/{asset_id}/stream",
            get(brands::stream_brand_media),
        )
        .route(
            "/api/v1/exports/{job_id}/download",
            get(exports::download_export),
        )
        .route("/api/v1/gfx/sandbox/{sandbox_id}", get(serve_gfx_sandbox))
        .route("/api/v1/ws", get(ws::ws_handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            stream_or_ws_auth_guard,
        ));

    let mut router = Router::new()
        .route("/api/v1/health", get(health))
        .merge(api_protected)
        .merge(stream_and_ws)
        .layer(cors);

    if state.inner.static_dir.is_some() {
        router = router.fallback(serve_static_ui);
    }

    router.with_state(state)
}

async fn serve_static_ui(State(state): State<AppState>, req: Request) -> Response {
    let Some(root) = &state.inner.static_dir else {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(axum::body::Body::empty())
            .unwrap();
    };
    let method = req.method().clone();
    if method != Method::GET && method != Method::HEAD {
        return Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .header(header::ALLOW, "GET, HEAD")
            .body(axum::body::Body::empty())
            .unwrap();
    }
    let req_path = req.uri().path().trim_start_matches('/');
    if req_path == "api"
        || req_path.starts_with("api/")
        || req_path == "mcp"
        || req_path.starts_with("mcp/")
    {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(axum::body::Body::empty())
            .unwrap();
    }

    let candidate = if req_path.is_empty() {
        root.join("index.html")
    } else {
        root.join(req_path)
    };
    let target = match tokio::fs::canonicalize(&candidate).await {
        Ok(canon) if canon.starts_with(root) && canon.is_file() => canon,
        _ => root.join("index.html"),
    };

    match tokio::fs::read(&target).await {
        Ok(bytes) => {
            let mime = static_mime(&target);
            let len = bytes.len();
            let body = if method == Method::HEAD {
                axum::body::Body::empty()
            } else {
                axum::body::Body::from(bytes)
            };
            let mut resp = Response::new(body);
            resp.headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
            resp.headers_mut().insert(
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            );
            let assets_dir = root.join("assets");
            if target.starts_with(&assets_dir) {
                resp.headers_mut().insert(
                    header::CACHE_CONTROL,
                    HeaderValue::from_static("public, max-age=31536000, immutable"),
                );
            } else if target.file_name().and_then(|n| n.to_str()) == Some("index.html") {
                resp.headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
            }
            if let Ok(len_val) = HeaderValue::from_str(&len.to_string()) {
                resp.headers_mut().insert(header::CONTENT_LENGTH, len_val);
            }
            resp
        }
        Err(_) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(axum::body::Body::empty())
            .unwrap(),
    }
}

fn static_mime(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "json" | "map" => "application/json",
        "webmanifest" => "application/manifest+json",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

const DEV_CORS_ORIGINS: &[&str] = &[
    "http://localhost:5173",
    "http://127.0.0.1:5173",
    "http://localhost:4173",
    "http://127.0.0.1:4173",
    "tauri://localhost",
    "http://tauri.localhost",
    "https://tauri.localhost",
];

fn effective_allowed_origins(state: &AppState) -> Vec<String> {
    let mut configured = state.inner.cors_origins.clone();
    if configured.is_empty()
        && let Ok(extra) = std::env::var("SOLARIS_CORS_ORIGINS")
    {
        configured = extra
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
    }

    let mut origins = Vec::new();
    if state.inner.is_loopback || configured.is_empty() {
        for o in DEV_CORS_ORIGINS {
            origins.push((*o).to_string());
        }
    }
    for o in configured {
        if !origins.iter().any(|existing| existing == &o) {
            origins.push(o);
        }
    }
    origins
}

fn build_cors_layer(state: &AppState) -> CorsLayer {
    let origins: Vec<HeaderValue> = effective_allowed_origins(state)
        .into_iter()
        .filter_map(|o| HeaderValue::from_str(&o).ok())
        .collect();

    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_credentials(true)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::RANGE,
            header::ACCEPT,
            HeaderName::from_static("sec-websocket-protocol"),
            HeaderName::from_static("mcp-session-id"),
        ])
        .expose_headers([HeaderName::from_static("mcp-session-id")])
}

fn is_origin_allowed(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true;
    };
    let origin = origin.trim();
    if effective_allowed_origins(state)
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(origin))
    {
        return true;
    }
    // Allow exact same-origin requests matching the Host header
    if let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) {
        let host = host.trim();
        let expected_http = format!("http://{host}");
        let expected_https = format!("https://{host}");
        if origin.eq_ignore_ascii_case(&expected_http)
            || origin.eq_ignore_ascii_case(&expected_https)
        {
            return true;
        }
    }
    false
}

fn is_host_allowed(state: &AppState, headers: &HeaderMap) -> bool {
    if !state.inner.is_loopback {
        return true;
    }
    let Some(host_hdr) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return true;
    };
    let host = host_hdr.trim();
    let bare = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        host.split(':').next().unwrap_or("")
    };
    crate::state::is_loopback_host(bare)
}

fn extract_session_cookie(headers: &HeaderMap) -> Option<&str> {
    let cookie_hdr = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in cookie_hdr.split(';') {
        if let Some(val) = part.trim().strip_prefix("solaris_session=") {
            return Some(val);
        }
    }
    None
}

async fn has_valid_bearer_or_session(
    state: &AppState,
    headers: &HeaderMap,
    expected: &str,
) -> bool {
    if let Some(auth_val) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        && constant_time_eq(auth_val.as_bytes(), expected.as_bytes())
    {
        return true;
    }

    if let Some(sid) = extract_session_cookie(headers)
        && state.verify_session_id(sid).await
    {
        return true;
    }

    false
}

async fn auth_guard(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if !is_host_allowed(&state, req.headers()) {
        return Err(StatusCode::FORBIDDEN);
    }
    if matches!(
        *req.method(),
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    ) && req.headers().contains_key(header::ORIGIN)
        && !is_origin_allowed(&state, req.headers())
    {
        return Err(StatusCode::FORBIDDEN);
    }
    if let Some(expected) = &state.inner.auth_token
        && !has_valid_bearer_or_session(&state, req.headers(), expected).await
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(next.run(req).await)
}

async fn stream_or_ws_auth_guard(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if !is_host_allowed(&state, req.headers()) {
        return Err(StatusCode::FORBIDDEN);
    }
    if req.uri().path() == "/api/v1/ws" && !is_origin_allowed(&state, req.headers()) {
        return Err(StatusCode::FORBIDDEN);
    }

    let Some(expected) = &state.inner.auth_token else {
        return Ok(next.run(req).await);
    };

    if has_valid_bearer_or_session(&state, req.headers(), expected).await {
        return Ok(next.run(req).await);
    }

    // Check ephemeral read-only media ticket in query string (?ticket=...)
    if let Some(query) = req.uri().query() {
        for pair in query.split('&') {
            if let Some(ticket) = pair.strip_prefix("ticket=")
                && state.verify_media_ticket(ticket).await
            {
                return Ok(next.run(req).await);
            }
        }
    }

    // Check WebSocket subprotocol ticket (Sec-WebSocket-Protocol: solaris.v1, solaris.ticket.<t>)
    if let Some(proto_hdr) = req
        .headers()
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|v| v.to_str().ok())
    {
        for proto in proto_hdr.split(',').map(str::trim) {
            if let Some(ticket) = proto.strip_prefix("solaris.ticket.")
                && state.verify_media_ticket(ticket).await
            {
                return Ok(next.run(req).await);
            }
        }
    }

    Err(StatusCode::UNAUTHORIZED)
}

#[derive(Serialize)]
struct SessionResponse {
    media_ticket: String,
    expires_in_sec: u64,
}

fn should_set_secure_cookie(state: &AppState, headers: &HeaderMap) -> bool {
    if !state.inner.is_loopback {
        return true;
    }
    if headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .next()
                .is_some_and(|p| p.trim().eq_ignore_ascii_case("https"))
        })
    {
        return true;
    }
    headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|o| o.trim().to_ascii_lowercase().starts_with("https://"))
}

async fn create_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, (StatusCode, String)> {
    let session_id = if state.inner.auth_token.is_some() {
        Some(state.issue_session_id().await)
    } else {
        None
    };
    let ticket = state.issue_media_ticket(session_id.clone()).await;
    let body = serde_json::to_string(&SessionResponse {
        media_ticket: ticket,
        expires_in_sec: 1800,
    })
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut resp = Response::new(axum::body::Body::from(body));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    if let Some(sid) = session_id {
        let secure_flag = if should_set_secure_cookie(&state, &headers) {
            "; Secure"
        } else {
            ""
        };
        if let Ok(cookie) = HeaderValue::from_str(&format!(
            "solaris_session={sid}; HttpOnly; SameSite=Lax; Path=/api/v1; Max-Age=1800{secure_flag}"
        )) {
            resp.headers_mut().insert(header::SET_COOKIE, cookie);
        }
    }
    Ok(resp)
}

async fn delete_session(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(sid) = extract_session_cookie(&headers) {
        state.revoke_session_id(sid).await;
    }
    let secure_flag = if should_set_secure_cookie(&state, &headers) {
        "; Secure"
    } else {
        ""
    };
    let mut resp = Response::new(axum::body::Body::empty());
    *resp.status_mut() = StatusCode::NO_CONTENT;
    if let Ok(cookie) = HeaderValue::from_str(&format!(
        "solaris_session=; HttpOnly; SameSite=Lax; Path=/api/v1; Max-Age=0{secure_flag}"
    )) {
        resp.headers_mut().insert(header::SET_COOKIE, cookie);
    }
    resp
}

#[derive(Deserialize)]
struct GfxSandboxRequest {
    html: String,
}

#[derive(Serialize)]
struct GfxSandboxResponse {
    id: String,
    path: String,
}

async fn register_gfx_sandbox(
    State(state): State<AppState>,
    Json(req): Json<GfxSandboxRequest>,
) -> Result<Json<GfxSandboxResponse>, (StatusCode, String)> {
    let id = state.register_gfx_sandbox(req.html).await?;
    Ok(Json(GfxSandboxResponse {
        path: format!("/api/v1/gfx/sandbox/{id}"),
        id,
    }))
}

async fn serve_gfx_sandbox(
    State(state): State<AppState>,
    AxumPath(sandbox_id): AxumPath<String>,
) -> Result<Response, (StatusCode, String)> {
    validate_safe_id(&sandbox_id)?;
    let Some(html) = state.get_gfx_sandbox(&sandbox_id).await else {
        return Err((
            StatusCode::NOT_FOUND,
            format!("GFX sandbox '{sandbox_id}' not found"),
        ));
    };
    let mut resp = Response::new(axum::body::Body::from(html));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    resp.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=300"),
    );
    resp.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src data:; font-src data:; sandbox allow-scripts",
        ),
    );
    Ok(resp)
}

async fn mcp_http_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Response {
    if !is_origin_allowed(&state, &headers) || !is_host_allowed(&state, &headers) {
        return Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(axum::body::Body::from(
                "MCP request rejected: disallowed Origin or Host header",
            ))
            .unwrap();
    }
    let is_json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| {
            ct.split(';')
                .next()
                .is_some_and(|mt| mt.trim().eq_ignore_ascii_case("application/json"))
        });
    if !is_json {
        return Response::builder()
            .status(StatusCode::UNSUPPORTED_MEDIA_TYPE)
            .body(axum::body::Body::from(
                "MCP requests require Content-Type: application/json",
            ))
            .unwrap();
    }
    let sessions = state.inner.mcp_sessions.clone();
    let backend: Arc<dyn SolarisBackend> = Arc::new(state);
    let server = McpServer::new(backend);
    mcp_post_handler(&server, &sessions, &headers, body).await
}

async fn mcp_http_delete(State(state): State<AppState>, headers: HeaderMap) -> StatusCode {
    if !is_origin_allowed(&state, &headers) || !is_host_allowed(&state, &headers) {
        return StatusCode::FORBIDDEN;
    }
    mcp_delete_handler(&state.inner.mcp_sessions, &headers).await
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    storage_backend: &'static str,
    toolchain: ToolchainCapabilities,
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        storage_backend: state.inner.storage.backend_kind(),
        toolchain: state.inner.capabilities.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use chrono::Utc;
    use std::path::PathBuf;
    use tower::ServiceExt;

    use crate::state::ServerConfig;
    use solaris_core_ir::{
        AudioSettings, BrandPlaybook, BrandReferenceKind, Clip, ClipKind, Project, TimelineCommand,
        Transform2D,
    };

    fn test_app(name: &str, auth_token: Option<&str>) -> (Router, PathBuf) {
        let dir = std::env::temp_dir().join(format!("solaris_srv_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let state = AppState::initialize(&ServerConfig {
            workspace_dir: dir.clone(),
            auth_token: auth_token.map(str::to_string),
            ..ServerConfig::default()
        })
        .expect("init state");
        (build_router(state), dir)
    }

    #[tokio::test]
    async fn health_and_project_op_undo_flow() {
        let (app, dir) = test_app("flow", None);

        let res = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/v1/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let create_res = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/v1/projects")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"name":"demo_short","aspect_preset":"vertical9x16"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_res.status(), StatusCode::CREATED);
        let bytes = axum::body::to_bytes(create_res.into_body(), usize::MAX)
            .await
            .unwrap();
        let created: Project = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(created.tracks.len(), 7);

        let op_body = serde_json::to_string(&serde_json::json!({
            "command": TimelineCommand::PlaceClip {
                track_id: "v1".to_string(),
                clip: Clip {
                    id: "c1".to_string(),
                    label: "Hook Clip".to_string(),
                    start_sec: 0.0,
                    duration_sec: 3.5,
                    source_in_sec: 0.0,
                    transform: Transform2D::default(),
                    audio: AudioSettings::default(),
                    payload: ClipKind::Media {
                        media_id: "m1".to_string(),
                        ken_burns: false,
                    }
                }
            },
            "meta": {
                "agent_id": "razor-01",
                "role": "razor"
            }
        }))
        .unwrap();

        let op_res = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri(format!("/api/v1/projects/{}/ops", created.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(op_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(op_res.status(), StatusCode::OK);

        let undo_res = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri(format!("/api/v1/projects/{}/undo", created.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(undo_res.status(), StatusCode::OK);
        let undo_bytes = axum::body::to_bytes(undo_res.into_body(), usize::MAX)
            .await
            .unwrap();
        let after_undo: Project = serde_json::from_slice(&undo_bytes).unwrap();
        let v1 = after_undo.tracks.iter().find(|t| t.id == "v1").unwrap();
        assert!(v1.clips.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn brand_reference_media_delete_cleans_lut_and_export_job_routes() {
        let (app, dir) = test_app("brand_export", None);

        let cube_path = dir.join("solar.cube");
        std::fs::write(
            &cube_path,
            "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n",
        )
        .unwrap();

        let body = serde_json::json!({
            "paths": [{ "path": cube_path.to_string_lossy() }],
            "notes": "Signature warm solar LUT"
        })
        .to_string();

        let brand_res = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/v1/brands/solaris-default/media")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(brand_res.status(), StatusCode::OK);
        let brand_bytes = axum::body::to_bytes(brand_res.into_body(), usize::MAX)
            .await
            .unwrap();
        let updated_brand: BrandPlaybook = serde_json::from_slice(&brand_bytes).unwrap();
        assert_eq!(updated_brand.reference_media.len(), 1);
        assert_eq!(
            updated_brand.reference_media[0].kind,
            BrandReferenceKind::LutCube
        );
        let asset_id = updated_brand.reference_media[0].id.clone();

        // Range streaming check on brand media
        let range_res = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri(format!(
                        "/api/v1/brands/solaris-default/media/{asset_id}/stream"
                    ))
                    .header(header::RANGE, "bytes=0-10")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(range_res.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            range_res.headers().get(header::CONTENT_LENGTH).unwrap(),
            "11"
        );

        let bad_range = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri(format!(
                        "/api/v1/brands/solaris-default/media/{asset_id}/stream"
                    ))
                    .header(header::RANGE, "bytes=999999-1000000")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bad_range.status(), StatusCode::RANGE_NOT_SATISFIABLE);

        let lut_res = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/v1/brands/solaris-default/lut")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(lut_res.status(), StatusCode::OK);

        // Delete LUT reference and ensure default_lut is cleared
        let del_res = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("DELETE")
                    .uri(format!("/api/v1/brands/solaris-default/media/{asset_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(del_res.status(), StatusCode::OK);
        let del_bytes = axum::body::to_bytes(del_res.into_body(), usize::MAX)
            .await
            .unwrap();
        let after_del: BrandPlaybook = serde_json::from_slice(&del_bytes).unwrap();
        assert!(after_del.default_lut.is_none());

        let create_res = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/v1/projects")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"name":"export_demo","aspect_preset":"vertical9x16"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let proj_bytes = axum::body::to_bytes(create_res.into_body(), usize::MAX)
            .await
            .unwrap();
        let created: Project = serde_json::from_slice(&proj_bytes).unwrap();

        let export_res = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri(format!("/api/v1/projects/{}/export", created.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"format":"mp4","quality":"standard","target_lufs":-14.0}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(export_res.status(), StatusCode::ACCEPTED);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn enforces_bearer_auth_session_tickets_and_blocks_id_traversal() {
        let (app, dir) = test_app("auth_sec", Some("secret-token-123"));

        // Unauthenticated API call fails
        let unauth = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/v1/projects")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);

        // Exchange bearer for session cookie + media ticket
        let sess = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/v1/auth/session")
                    .header(header::AUTHORIZATION, "Bearer secret-token-123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(sess.status(), StatusCode::OK);
        let set_cookie = sess
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(set_cookie.contains("solaris_session="));
        assert!(set_cookie.contains("Max-Age=1800"));
        assert!(!set_cookie.contains("secret-token-123"));
        let cookie_pair = set_cookie.split(';').next().unwrap().to_string();

        // Opaque session cookie authenticates API requests
        let cookie_auth = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/v1/projects")
                    .header(header::COOKIE, &cookie_pair)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(cookie_auth.status(), StatusCode::OK);

        // MCP rejects missing application/json content-type and disallowed Origin
        let mcp_bad_ct = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header(header::AUTHORIZATION, "Bearer secret-token-123")
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(mcp_bad_ct.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

        let mcp_bad_origin = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header(header::AUTHORIZATION, "Bearer secret-token-123")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::ORIGIN, "https://evil.example")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(mcp_bad_origin.status(), StatusCode::FORBIDDEN);

        // WebSocket rejects disallowed Origin
        let ws_bad_origin = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/v1/ws")
                    .header(header::AUTHORIZATION, "Bearer secret-token-123")
                    .header(header::ORIGIN, "https://evil.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ws_bad_origin.status(), StatusCode::FORBIDDEN);

        // Path traversal in brand ID is rejected
        let bad_brand = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/v1/brands")
                    .header(header::AUTHORIZATION, "Bearer secret-token-123")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "id": "../../evil",
                            "name": "Evil",
                            "brand_md": "",
                            "tokens": {
                                "bg": "#080404",
                                "surface": "#1A0E0E",
                                "ink": "#F7F5EF",
                                "accent": "#F05A00",
                                "accent_secondary": "#F7B733",
                                "font_display": "Sora",
                                "font_body": "Sora"
                            },
                            "pacing_rules": "",
                            "updated_at": Utc::now()
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bad_brand.status(), StatusCode::BAD_REQUEST);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn persists_prompt_and_enforces_track_lock_routes() {
        let (app, dir) = test_app("locks_prompt", None);

        let create_res = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/v1/projects")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"name":"p1","aspect_preset":"vertical9x16","prompt":"Edit a 20s hook reel"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_res.status(), StatusCode::CREATED);
        let bytes = axum::body::to_bytes(create_res.into_body(), usize::MAX)
            .await
            .unwrap();
        let created: Project = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(created.prompt, "Edit a 20s hook reel");

        let lock_res = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri(format!("/api/v1/projects/{}/locks/t1", created.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"agent_id":"caps-01","role":"caps"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(lock_res.status(), StatusCode::NO_CONTENT);

        let conflict_res = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri(format!("/api/v1/projects/{}/locks/t1", created.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"agent_id":"razor-01","role":"razor"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(conflict_res.status(), StatusCode::CONFLICT);

        let release_res = app
            .oneshot(
                HttpRequest::builder()
                    .method("DELETE")
                    .uri(format!(
                        "/api/v1/projects/{}/locks/t1?agent_id=caps-01",
                        created.id
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(release_res.status(), StatusCode::NO_CONTENT);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn agent_status_skills_mcp_http_and_local_swarm_run() {
        let (app, dir) = test_app("phase3_agent_mcp", None);

        let status_res = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/v1/agent/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(status_res.status(), StatusCode::OK);
        let status_bytes = axum::body::to_bytes(status_res.into_body(), usize::MAX)
            .await
            .unwrap();
        let status_json: serde_json::Value = serde_json::from_slice(&status_bytes).unwrap();
        assert_eq!(status_json["skills_count"], 6);
        assert_eq!(status_json["mcp_http_path"], "/mcp");

        // MCP Streamable HTTP initialize + tools/list + project.create + session invalidation
        let mcp_init = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(mcp_init.status(), StatusCode::OK);
        let mcp_sid = mcp_init
            .headers()
            .get("mcp-session-id")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let mcp_create = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header("mcp-session-id", &mcp_sid)
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"project.create","arguments":{"name":"mcp_proj"}}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(mcp_create.status(), StatusCode::OK);
        let mcp_bytes = axum::body::to_bytes(mcp_create.into_body(), usize::MAX)
            .await
            .unwrap();
        let mcp_val: serde_json::Value = serde_json::from_slice(&mcp_bytes).unwrap();
        let pid = mcp_val["result"]["structuredContent"]["id"]
            .as_str()
            .unwrap()
            .to_string();

        // DELETE /mcp invalidates the tracked session
        let mcp_del = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("DELETE")
                    .uri("/mcp")
                    .header("mcp-session-id", &mcp_sid)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(mcp_del.status(), StatusCode::NO_CONTENT);

        let mcp_after_del = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header("mcp-session-id", &mcp_sid)
                    .body(Body::from(r#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(mcp_after_del.status(), StatusCode::NOT_FOUND);

        // Runtime BYOK OpenRouter credentials update + clear (raw key never echoed)
        let cred_set = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/v1/agent/credentials")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"api_key":"sk-or-v1-test-secret-key","default_model":"google/gemini-2.5-flash"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(cred_set.status(), StatusCode::OK);
        let cred_body = axum::body::to_bytes(cred_set.into_body(), usize::MAX)
            .await
            .unwrap();
        let cred_text = String::from_utf8_lossy(&cred_body);
        assert!(!cred_text.contains("sk-or-v1-test-secret-key"));
        let cred_json: serde_json::Value = serde_json::from_slice(&cred_body).unwrap();
        assert_eq!(cred_json["openrouter_configured"], true);
        assert_eq!(cred_json["default_model"], "google/gemini-2.5-flash");

        let cred_del = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("DELETE")
                    .uri("/api/v1/agent/credentials")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(cred_del.status(), StatusCode::OK);
        let cred_del_json: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(cred_del.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(cred_del_json["openrouter_configured"], false);

        // Trigger local fallback swarm run
        let swarm_res = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri(format!("/api/v1/projects/{pid}/swarm/run"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"prompt":"Assemble a 9:16 edit","allow_local_fallback":true}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(swarm_res.status(), StatusCode::ACCEPTED);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn templates_asset_pack_import_export_and_qa_routes() {
        let (app, dir) = test_app("phase4_tpl_pack_qa", None);

        // 1. Templates list
        let tpl_res = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/v1/templates")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(tpl_res.status(), StatusCode::OK);
        let tpl_bytes = axum::body::to_bytes(tpl_res.into_body(), usize::MAX)
            .await
            .unwrap();
        let tpl_list: Vec<serde_json::Value> = serde_json::from_slice(&tpl_bytes).unwrap();
        assert_eq!(tpl_list.len(), 2);

        // 2. Create project from template
        let create_res = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/v1/projects")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"name":"from_tpl","template_id":"talking-head-short"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_res.status(), StatusCode::CREATED);
        let proj_bytes = axum::body::to_bytes(create_res.into_body(), usize::MAX)
            .await
            .unwrap();
        let proj: Project = serde_json::from_slice(&proj_bytes).unwrap();
        assert!(
            !proj
                .tracks
                .iter()
                .find(|t| t.id == "g1")
                .unwrap()
                .clips
                .is_empty()
        );

        // 3. Export asset pack
        let exp_res = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/v1/assets/export")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(exp_res.status(), StatusCode::OK);
        let exp_bytes = axum::body::to_bytes(exp_res.into_body(), usize::MAX)
            .await
            .unwrap();
        let pack_json: serde_json::Value = serde_json::from_slice(&exp_bytes).unwrap();
        assert_eq!(pack_json["schema_version"], "solaris.asset-pack/v1");
        assert_eq!(pack_json["assets"].as_array().unwrap().len(), 5);

        // 4. Reject unsafe <script src=...> on import
        let bad_pack = serde_json::json!({
            "schema_version": "solaris.asset-pack/v1",
            "pack_id": "bad-pack",
            "name": "Bad Pack",
            "assets": [{
                "id": "evil-card",
                "name": "Evil",
                "category": "cards",
                "tags": [],
                "props_schema": {},
                "default_props": {},
                "html_source": "<script src=\"https://evil.test/x.js\"></script>",
                "default_duration_sec": 2.0,
                "created_at": Utc::now()
            }]
        })
        .to_string();
        let bad_imp = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/v1/assets/import")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(bad_pack))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bad_imp.status(), StatusCode::BAD_REQUEST);

        // 5. Run QA endpoint on templated project
        let qa_res = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri(format!("/api/v1/projects/{}/qa", proj.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"auto_fix":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(qa_res.status(), StatusCode::OK);
        let qa_bytes = axum::body::to_bytes(qa_res.into_body(), usize::MAX)
            .await
            .unwrap();
        let qa_json: serde_json::Value = serde_json::from_slice(&qa_bytes).unwrap();
        // A bare template scaffold has only partial G1 beats and no V1/V2 footage yet,
        // so QA should return a structured report with checks and a non-contradictory score.
        assert!(qa_json["score"].as_u64().is_some());
        assert!(
            qa_json["checks"]
                .as_array()
                .is_some_and(|checks| !checks.is_empty())
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn export_resolves_media_paths_and_blocks_external_reads_when_confined() {
        let dir =
            std::env::temp_dir().join(format!("solaris_srv_export_confine_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let state = AppState::initialize(&ServerConfig {
            workspace_dir: dir.clone(),
            allow_external_media_reads: false,
            ..ServerConfig::default()
        })
        .expect("init state");
        let app = build_router(state.clone());

        let create_res = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/v1/projects")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"name":"confine_export"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_res.status(), StatusCode::CREATED);
        let created: Project = serde_json::from_slice(
            &axum::body::to_bytes(create_res.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();

        // AddMedia op rejects external SSRF / host path when allow_external_media_reads == false
        let bad_media_op = serde_json::to_string(&serde_json::json!({
            "command": TimelineCommand::AddMedia {
                asset: solaris_core_ir::MediaAsset {
                    id: "ext1".into(),
                    name: "secret.mp4".into(),
                    kind: solaris_core_ir::MediaKind::Video,
                    role_hint: "a_roll".into(),
                    path: "http://169.254.169.254/latest/meta-data/".into(),
                    added_at: Utc::now(),
                    intelligence: solaris_core_ir::MediaIntelligenceManifest::default(),
                }
            },
            "meta": { "agent_id": "user", "role": "user" }
        }))
        .unwrap();
        let bad_media_res = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri(format!("/api/v1/projects/{}/ops", created.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(bad_media_op))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bad_media_res.status(), StatusCode::FORBIDDEN);

        // Even if a persisted project contains an external path referenced by a clip,
        // export resolves every media_pool path through AssetStorage and fails with 403 before FFmpeg runs.
        {
            let mut guard = state.inner.projects.write().await;
            let engine = guard.get_mut(&created.id).unwrap();
            engine
                .apply(
                    TimelineCommand::AddMedia {
                        asset: solaris_core_ir::MediaAsset {
                            id: "ext_legacy".into(),
                            name: "outside.mp4".into(),
                            kind: solaris_core_ir::MediaKind::Video,
                            role_hint: "a_roll".into(),
                            path: "/etc/hosts".into(),
                            added_at: Utc::now(),
                            intelligence: solaris_core_ir::MediaIntelligenceManifest::default(),
                        },
                    },
                    solaris_core_ir::CommandMeta::default(),
                )
                .unwrap();
            engine
                .apply(
                    TimelineCommand::PlaceClip {
                        track_id: "v1".into(),
                        clip: Clip {
                            id: "c_ext".into(),
                            label: "External".into(),
                            start_sec: 0.0,
                            duration_sec: 2.0,
                            source_in_sec: 0.0,
                            transform: Transform2D::default(),
                            audio: AudioSettings::default(),
                            payload: ClipKind::Media {
                                media_id: "ext_legacy".into(),
                                ken_burns: false,
                            },
                        },
                    },
                    solaris_core_ir::CommandMeta::default(),
                )
                .unwrap();
        }

        let blocked_export = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri(format!("/api/v1/projects/{}/export", created.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"format":"mp4","quality":"standard","target_lufs":-14.0}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(blocked_export.status(), StatusCode::FORBIDDEN);

        // Workspace-confined media path resolves cleanly and allows export enqueue
        let safe_rel = state
            .inner
            .storage
            .save_bytes("projects/test/clip.mp4", b"fake-mp4-bytes")
            .expect("store workspace media")
            .to_string_lossy()
            .to_string();
        {
            let mut guard = state.inner.projects.write().await;
            let engine = guard.get_mut(&created.id).unwrap();
            engine
                .apply(
                    TimelineCommand::DeleteClip {
                        track_id: "v1".into(),
                        clip_id: "c_ext".into(),
                    },
                    solaris_core_ir::CommandMeta::default(),
                )
                .unwrap();
            engine
                .apply(
                    TimelineCommand::AddMedia {
                        asset: solaris_core_ir::MediaAsset {
                            id: "safe1".into(),
                            name: "clip.mp4".into(),
                            kind: solaris_core_ir::MediaKind::Video,
                            role_hint: "a_roll".into(),
                            path: safe_rel,
                            added_at: Utc::now(),
                            intelligence: solaris_core_ir::MediaIntelligenceManifest::default(),
                        },
                    },
                    solaris_core_ir::CommandMeta::default(),
                )
                .unwrap();
            engine
                .apply(
                    TimelineCommand::PlaceClip {
                        track_id: "v1".into(),
                        clip: Clip {
                            id: "c_safe".into(),
                            label: "Safe".into(),
                            start_sec: 0.0,
                            duration_sec: 2.0,
                            source_in_sec: 0.0,
                            transform: Transform2D::default(),
                            audio: AudioSettings::default(),
                            payload: ClipKind::Media {
                                media_id: "safe1".into(),
                                ken_burns: false,
                            },
                        },
                    },
                    solaris_core_ir::CommandMeta::default(),
                )
                .unwrap();
        }

        let allowed_export = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri(format!("/api/v1/projects/{}/export", created.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"format":"mp4","quality":"standard","target_lufs":-14.0}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(allowed_export.status(), StatusCode::ACCEPTED);

        let _ = std::fs::remove_dir_all(&dir);
    }

    async fn ws_connect(addr: std::net::SocketAddr) -> tokio::net::TcpStream {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let req = format!(
            "GET /api/v1/ws HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: solaris.v1\r\n\r\n"
        );
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut header_buf = Vec::new();
        let mut byte = [0u8; 1];
        while !header_buf.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).await.unwrap();
            header_buf.push(byte[0]);
        }
        let header_text = String::from_utf8_lossy(&header_buf);
        assert!(
            header_text.starts_with("HTTP/1.1 101"),
            "expected 101 Switching Protocols, got: {header_text}"
        );
        stream
    }

    async fn ws_read_frame(stream: &mut tokio::net::TcpStream) -> (u8, Vec<u8>) {
        use tokio::io::AsyncReadExt;
        let mut hdr = [0u8; 2];
        stream.read_exact(&mut hdr).await.unwrap();
        let opcode = hdr[0] & 0x0f;
        let mut len = (hdr[1] & 0x7f) as usize;
        if len == 126 {
            let mut ext = [0u8; 2];
            stream.read_exact(&mut ext).await.unwrap();
            len = u16::from_be_bytes(ext) as usize;
        } else if len == 127 {
            let mut ext = [0u8; 8];
            stream.read_exact(&mut ext).await.unwrap();
            len = u64::from_be_bytes(ext) as usize;
        }
        let mut payload = vec![0u8; len];
        if len > 0 {
            stream.read_exact(&mut payload).await.unwrap();
        }
        (opcode, payload)
    }

    #[tokio::test]
    async fn websocket_handles_events_lag_and_clean_close() {
        use tokio::io::AsyncWriteExt;

        let dir = std::env::temp_dir().join(format!("solaris_srv_ws_lag_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let state = AppState::initialize(&ServerConfig {
            workspace_dir: dir.clone(),
            ..ServerConfig::default()
        })
        .expect("init state");
        let router = build_router(state.clone());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server_task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });

        // 1. Connect normal client, receive broadcast event, send masked Close frame
        let mut client_a = ws_connect(addr).await;
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        state.emit(solaris_core_ir::EngineEvent::CursorPulse {
            project_id: "p_ws".into(),
            agent_id: "razor-01".into(),
            role: solaris_core_ir::AgentRole::Razor,
            ui_anchor: "timeline.track.v1".into(),
            label: "Inspecting hook".into(),
        });
        let (op_a, payload_a) = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            ws_read_frame(&mut client_a),
        )
        .await
        .expect("ws event frame timeout");
        assert_eq!(op_a, 0x1, "expected text frame");
        assert!(String::from_utf8_lossy(&payload_a).contains("Inspecting hook"));

        // Send masked RFC 6455 Close frame (FIN=1, opcode=0x8, MASK=1, len=0, mask=0)
        client_a
            .write_all(&[0x88, 0x80, 0x00, 0x00, 0x00, 0x00])
            .await
            .unwrap();
        drop(client_a);

        // 2. Connect slow client, flood >2048 broadcast events to trigger RecvError::Lagged,
        //    and confirm server emits a WebSocket Close frame cleanly.
        let mut client_b = ws_connect(addr).await;
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        for i in 0..2500 {
            state.emit(solaris_core_ir::EngineEvent::CursorPulse {
                project_id: "p_ws".into(),
                agent_id: "caps-01".into(),
                role: solaris_core_ir::AgentRole::Caps,
                ui_anchor: "timeline.track.t1".into(),
                label: format!("burst-{i}"),
            });
        }
        let mut saw_close = false;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(
                std::time::Duration::from_millis(500),
                ws_read_frame(&mut client_b),
            )
            .await
            {
                Ok((0x8, _)) => {
                    saw_close = true;
                    break;
                }
                Ok(_) => continue,
                Err(_) => break,
            }
        }
        assert!(
            saw_close,
            "expected server to send WebSocket Close frame on broadcast lag"
        );

        server_task.abort();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn websocket_closes_gracefully_on_shutdown_cancel() {
        let dir =
            std::env::temp_dir().join(format!("solaris_srv_ws_shutdown_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let state = AppState::initialize(&ServerConfig {
            workspace_dir: dir.clone(),
            ..ServerConfig::default()
        })
        .expect("init state");
        let router = build_router(state.clone());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let shutdown = state.inner.shutdown.clone();
        let server_task = tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    shutdown.cancelled().await;
                })
                .await;
        });

        let mut client = ws_connect(addr).await;
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        state.inner.shutdown.cancel();

        let (op, _) = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            ws_read_frame(&mut client),
        )
        .await
        .expect("expected Close frame immediately upon shutdown.cancel()");
        assert_eq!(op, 0x8, "expected WebSocket Close frame");

        drop(client);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), server_task)
            .await
            .expect("server should shut down cleanly after WS closes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn dns_rebinding_csrf_content_type_and_logout_revokes_tickets() {
        let (app, dir) = test_app("sec_wave2", Some("secret-wave2-token"));

        // 1. DNS rebinding Host rejected on API and WS
        let bad_host_api = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/v1/projects")
                    .header(header::HOST, "attacker.example:41731")
                    .header(header::AUTHORIZATION, "Bearer secret-wave2-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bad_host_api.status(), StatusCode::FORBIDDEN);

        let bad_host_ws = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/v1/ws")
                    .header(header::HOST, "attacker.example:41731")
                    .header(header::AUTHORIZATION, "Bearer secret-wave2-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bad_host_ws.status(), StatusCode::FORBIDDEN);

        // 2. Cross-site Origin rejected on state-changing API routes
        let bad_origin_post = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/v1/projects")
                    .header(header::AUTHORIZATION, "Bearer secret-wave2-token")
                    .header(header::ORIGIN, "https://evil.example")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"name":"csrf"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bad_origin_post.status(), StatusCode::FORBIDDEN);

        // 3. Create project, then test 415 Unsupported Media Type for text/plain on import_media & brand media
        let create_res = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/v1/projects")
                    .header(header::AUTHORIZATION, "Bearer secret-wave2-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"name":"ct_check"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_res.status(), StatusCode::CREATED);
        let created: Project = serde_json::from_slice(
            &axum::body::to_bytes(create_res.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();

        let bad_ct_media = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri(format!("/api/v1/projects/{}/media/import", created.id))
                    .header(header::AUTHORIZATION, "Bearer secret-wave2-token")
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(Body::from(r#"{"paths":[]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bad_ct_media.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

        let bad_ct_brand = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/v1/brands/solaris-default/media")
                    .header(header::AUTHORIZATION, "Bearer secret-wave2-token")
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(Body::from(r#"{"paths":[]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bad_ct_brand.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

        // 4. Session login -> register GFX sandbox -> verify ticket works -> logout -> verify ticket revoked
        let sess_res = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/v1/auth/session")
                    .header(header::AUTHORIZATION, "Bearer secret-wave2-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(sess_res.status(), StatusCode::OK);
        let cookie_pair = sess_res
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let sess_json: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(sess_res.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        let ticket = sess_json["media_ticket"].as_str().unwrap().to_string();

        let sb_post = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/v1/gfx/sandbox")
                    .header(header::COOKIE, &cookie_pair)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"html":"<!doctype html><html><body><div>Solaris</div></body></html>"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(sb_post.status(), StatusCode::OK);
        let sb_json: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(sb_post.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        let sb_path = sb_json["path"].as_str().unwrap().to_string();

        // Unauthenticated GET to sandbox fails
        let sb_unauth = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri(&sb_path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(sb_unauth.status(), StatusCode::UNAUTHORIZED);

        // Authenticated GET via ?ticket= succeeds with strict sandbox CSP
        let sb_get = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri(format!("{sb_path}?ticket={ticket}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(sb_get.status(), StatusCode::OK);
        assert_eq!(
            sb_get
                .headers()
                .get(header::CONTENT_SECURITY_POLICY)
                .unwrap(),
            "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src data:; font-src data:; sandbox allow-scripts"
        );
        assert_eq!(
            sb_get
                .headers()
                .get(header::X_CONTENT_TYPE_OPTIONS)
                .unwrap(),
            "nosniff"
        );

        // Logout revokes session AND linked media_ticket, and sets Secure when X-Forwarded-Proto: https
        let logout = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("DELETE")
                    .uri("/api/v1/auth/session")
                    .header(header::COOKIE, &cookie_pair)
                    .header("x-forwarded-proto", "https")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(logout.status(), StatusCode::NO_CONTENT);
        let clear_cookie = logout
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(clear_cookie.contains("Max-Age=0"));
        assert!(clear_cookie.contains("; Secure"));

        // Ticket issued by that session is now immediately invalid
        let sb_after_logout = app
            .oneshot(
                HttpRequest::builder()
                    .uri(format!("{sb_path}?ticket={ticket}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(sb_after_logout.status(), StatusCode::UNAUTHORIZED);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn patch_project_metadata_and_delete_media_routes() {
        let (app, dir) = test_app("meta_del_media", None);

        let create_res = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/v1/projects")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"name":"Initial Name","prompt":"Old prompt"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_res.status(), StatusCode::CREATED);
        let created: Project = serde_json::from_slice(
            &axum::body::to_bytes(create_res.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();

        // PATCH metadata updates name & prompt and increments revision
        let patch_res = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("PATCH")
                    .uri(format!("/api/v1/projects/{}", created.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"name":"  Renamed Edit  ","prompt":"New brief"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(patch_res.status(), StatusCode::OK);
        let patched: Project = serde_json::from_slice(
            &axum::body::to_bytes(patch_res.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(patched.name, "Renamed Edit");
        assert_eq!(patched.prompt, "New brief");
        assert!(patched.revision > created.revision);

        // Blank name rejected with 400
        let bad_patch = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("PATCH")
                    .uri(format!("/api/v1/projects/{}", created.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"name":"   "}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bad_patch.status(), StatusCode::BAD_REQUEST);

        // Create an uploaded file inside workspace uploads/ and import it, then DELETE it
        let upload_dir = dir.join("uploads").join(&created.id);
        std::fs::create_dir_all(&upload_dir).unwrap();
        let media_file = upload_dir.join("sample.mp4");
        std::fs::write(&media_file, b"fake-video-bytes").unwrap();

        let imp_res = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri(format!("/api/v1/projects/{}/media/import", created.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "paths": [{ "path": media_file.to_string_lossy() }]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(imp_res.status(), StatusCode::OK);
        let imported: Vec<solaris_core_ir::MediaAsset> = serde_json::from_slice(
            &axum::body::to_bytes(imp_res.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(imported.len(), 1);
        let mid = imported[0].id.clone();

        let del_res = app
            .oneshot(
                HttpRequest::builder()
                    .method("DELETE")
                    .uri(format!("/api/v1/projects/{}/media/{mid}", created.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(del_res.status(), StatusCode::OK);
        let after_del: Project = serde_json::from_slice(
            &axum::body::to_bytes(del_res.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(after_del.media_pool.is_empty());
        assert!(!media_file.exists(), "uploaded file should be deleted");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extracts_flate_compressed_pdf_streams() {
        use flate2::Compression;
        use flate2::write::ZlibEncoder;
        use std::io::Write;

        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(b"BT /F1 12 Tf (Solaris Compressed Brief) Tj ET")
            .unwrap();
        let compressed = enc.finish().unwrap();

        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.4\n1 0 obj\n<< /Filter /FlateDecode >>\nstream\n");
        pdf.extend_from_slice(&compressed);
        pdf.extend_from_slice(b"\nendstream\nendobj\n%%EOF");

        let extracted = crate::backend::extract_pdf_text(&pdf);
        assert!(
            extracted.contains("Solaris Compressed Brief"),
            "expected decompressed text, got: {extracted:?}"
        );
    }

    #[tokio::test]
    async fn logs_endpoint_lists_filters_redacts_secrets_and_clears() {
        let dir = std::env::temp_dir().join(format!("solaris_srv_logs_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let state = AppState::initialize(&ServerConfig {
            workspace_dir: dir.clone(),
            ..ServerConfig::default()
        })
        .expect("init state");

        state.push_log(solaris_core_ir::LogEntry {
            id: "log-1".to_string(),
            timestamp: Utc::now(),
            level: solaris_core_ir::LogLevel::Error,
            source: "openrouter".to_string(),
            project_id: Some("proj-a".to_string()),
            run_id: Some("run-1".to_string()),
            agent_id: Some("director-01".to_string()),
            message: "Key sk-or-v1-secret123456 failed with Bearer tok_999".to_string(),
            details: Some("raw payload with sk-or-v1-anothersecret".to_string()),
        });
        state.push_log(solaris_core_ir::LogEntry {
            id: "log-2".to_string(),
            timestamp: Utc::now(),
            level: solaris_core_ir::LogLevel::Info,
            source: "swarm".to_string(),
            project_id: Some("proj-b".to_string()),
            run_id: None,
            agent_id: None,
            message: "Other project info".to_string(),
            details: None,
        });

        let app = build_router(state);

        let res = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/v1/logs?project_id=proj-a")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let items: Vec<solaris_core_ir::LogEntry> = serde_json::from_slice(
            &axum::body::to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(items.len(), 1);
        assert!(!items[0].message.contains("secret123456"));
        assert!(!items[0].message.contains("tok_999"));
        assert!(items[0].message.contains("<redacted-key>"));
        assert!(items[0].message.contains("Bearer <redacted>"));
        assert!(
            !items[0]
                .details
                .as_deref()
                .unwrap()
                .contains("anothersecret")
        );

        let del = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("DELETE")
                    .uri("/api/v1/logs?project_id=proj-a")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(del.status(), StatusCode::NO_CONTENT);

        let after = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/v1/logs?project_id=proj-a")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let remaining: Vec<solaris_core_ir::LogEntry> = serde_json::from_slice(
            &axum::body::to_bytes(after.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(remaining.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn agent_plan_endpoint_generates_custom_plan_and_validates_prompt() {
        let dir = std::env::temp_dir().join(format!("solaris_srv_plan_{}", uuid::Uuid::new_v4()));
        let state = AppState::initialize(&ServerConfig {
            workspace_dir: dir.clone(),
            ..ServerConfig::default()
        })
        .expect("init state");
        let app = build_router(state);

        // Empty prompt rejected with 400
        let bad = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/v1/agent/plan")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"prompt":"   "}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

        // Valid prompt returns a structured, skill-grounded creative plan
        let ok = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/v1/agent/plan")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"prompt":"Create a 22s 16:9 kinetic launch video for a Rust compiler","aspect_preset":"landscape16x9"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(ok.into_body(), usize::MAX)
            .await
            .unwrap();
        let plan: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(plan["production_mode"], "programmatic");
        assert!(plan["beats"].as_array().is_some_and(|b| !b.is_empty()));
        assert!(
            plan["skills_consulted"]
                .as_array()
                .is_some_and(|s| s.iter().any(|v| v == "director-core"))
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
