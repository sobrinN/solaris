use std::path::{Path, PathBuf};

use axum::Json;
use axum::body::Body;
use axum::extract::{Multipart, Path as AxumPath, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use chrono::Utc;
use serde::Deserialize;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use solaris_core_ir::{
    CommandMeta, EngineEvent, MediaAsset, MediaIntelligenceManifest, MediaKind, Project,
    TimelineCommand,
};
use solaris_media_engine::{generate_proxy_with_encoders, probe_and_analyze_fast};

use crate::state::{AppState, map_core_error, map_storage_error, validate_safe_id};

const MAX_JSON_IMPORT_BYTES: usize = 2 * 1024 * 1024; // 2 MiB cap for non-multipart JSON body

#[derive(Deserialize)]
pub struct LocalMediaPathImport {
    pub paths: Vec<LocalPathItem>,
}

#[derive(Deserialize)]
pub struct LocalPathItem {
    pub path: String,
    #[serde(default)]
    pub role_hint: Option<String>,
}

pub async fn stream_multipart_field_to_workspace(
    workspace_dir: &Path,
    rel_key: &str,
    mut field: axum::extract::multipart::Field<'_>,
) -> Result<PathBuf, (StatusCode, String)> {
    let rel = Path::new(rel_key);
    if rel.is_absolute()
        || rel.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "invalid relative storage key".to_string(),
        ));
    }
    let dest = workspace_dir.join(rel);
    if let Some(parent) = dest.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    let mut file = File::create(&dest)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let write_res: Result<(), (StatusCode, String)> = async {
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
        {
            file.write_all(&chunk)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        }
        file.flush()
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        Ok(())
    }
    .await;

    if let Err(err) = write_res {
        drop(file);
        let _ = tokio::fs::remove_file(&dest).await;
        return Err(err);
    }

    Ok(dest)
}

pub async fn import_media(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    req: Request,
) -> Result<Json<Vec<MediaAsset>>, (StatusCode, String)> {
    validate_safe_id(&id)?;
    {
        let guard = state.inner.projects.read().await;
        if !guard.contains_key(&id) {
            return Err((StatusCode::NOT_FOUND, format!("project '{id}' not found")));
        }
    }

    let content_type = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();

    let mut pending_assets: Vec<(MediaAsset, PathBuf)> = Vec::new();

    if content_type.starts_with("multipart/form-data") {
        let mut multipart =
            <Multipart as axum::extract::FromRequest<AppState>>::from_request(req, &state)
                .await
                .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

        let mut requested_role_hint: Option<String> = None;
        while let Some(field) = multipart
            .next_field()
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
        {
            let field_name = field.name().unwrap_or("").to_string();
            let Some(raw_file_name) = field.file_name().map(|s| s.to_string()) else {
                if field_name == "role_hint"
                    && let Ok(text) = field.text().await
                {
                    let trimmed = text.trim().to_string();
                    if !trimmed.is_empty() {
                        requested_role_hint = Some(trimmed);
                    }
                }
                continue;
            };
            let media_id = Uuid::new_v4().to_string();
            let safe_name = sanitize_file_name(&raw_file_name);
            let rel_key = format!("uploads/{id}/{media_id}_{safe_name}");
            let saved_path =
                stream_multipart_field_to_workspace(&state.inner.workspace_dir, &rel_key, field)
                    .await?;
            let storage = state.inner.storage.clone();
            let rel_clone = rel_key.clone();
            let local_clone = saved_path.clone();
            let synced_path =
                tokio::task::spawn_blocking(move || storage.sync_file(&rel_clone, &local_clone))
                    .await
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

            let kind = infer_media_kind(&safe_name);
            let role_hint = requested_role_hint
                .clone()
                .unwrap_or_else(|| infer_default_role_hint(kind, &safe_name).to_string());
            let asset = MediaAsset {
                id: media_id,
                name: safe_name,
                kind,
                role_hint,
                path: synced_path,
                added_at: Utc::now(),
                intelligence: MediaIntelligenceManifest::default(),
            };
            pending_assets.push((asset, saved_path));
        }
    } else {
        let is_json = content_type
            .split(';')
            .next()
            .is_some_and(|mt| mt.trim().eq_ignore_ascii_case("application/json"));
        if !is_json {
            return Err((
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "expected Content-Type: application/json or multipart/form-data".to_string(),
            ));
        }
        let bytes = axum::body::to_bytes(req.into_body(), MAX_JSON_IMPORT_BYTES)
            .await
            .map_err(|e| (StatusCode::PAYLOAD_TOO_LARGE, e.to_string()))?;
        let body: LocalMediaPathImport =
            serde_json::from_slice(&bytes).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

        for item in body.paths {
            let resolved = state
                .resolve_readable_path_async(item.path)
                .await
                .map_err(map_storage_error)?;
            let name = resolved
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("media")
                .to_string();
            let kind = infer_media_kind(&name);
            let asset = MediaAsset {
                id: Uuid::new_v4().to_string(),
                name: name.clone(),
                kind,
                role_hint: item
                    .role_hint
                    .unwrap_or_else(|| infer_default_role_hint(kind, &name).to_string()),
                path: resolved.to_string_lossy().to_string(),
                added_at: Utc::now(),
                intelligence: MediaIntelligenceManifest::default(),
            };
            pending_assets.push((asset, resolved));
        }
    }

    let mut added = Vec::with_capacity(pending_assets.len());
    let project_snapshot = {
        let mut guard = state.inner.projects.write().await;
        let engine = guard
            .get_mut(&id)
            .ok_or_else(|| (StatusCode::NOT_FOUND, format!("project '{id}' not found")))?;

        for (asset, _) in &pending_assets {
            let (_, event) = engine
                .apply(
                    TimelineCommand::AddMedia {
                        asset: asset.clone(),
                    },
                    CommandMeta::default(),
                )
                .map_err(map_core_error)?;
            state.emit(event);
            added.push(asset.clone());
        }
        engine.project().clone()
    };
    state.persist_project_locked(&project_snapshot).await;

    // Two-stage background analysis: fast probe/peaks/silences/beats/LUFS first, then proxy video.
    for (asset, path) in pending_assets {
        spawn_media_analysis_task(state.clone(), id.clone(), asset, path);
    }

    Ok(Json(added))
}

pub async fn delete_media(
    State(state): State<AppState>,
    AxumPath((id, media_id)): AxumPath<(String, String)>,
) -> Result<Json<Project>, (StatusCode, String)> {
    validate_safe_id(&id)?;
    validate_safe_id(&media_id)?;

    let (project, removed_asset) = {
        let mut guard = state.inner.projects.write().await;
        let engine = guard
            .get_mut(&id)
            .ok_or_else(|| (StatusCode::NOT_FOUND, format!("project '{id}' not found")))?;
        let existing = engine
            .project()
            .media_pool
            .iter()
            .find(|m| m.id == media_id)
            .cloned();
        let (_, event) = engine
            .apply(
                TimelineCommand::RemoveMedia {
                    media_id: media_id.clone(),
                },
                CommandMeta::default(),
            )
            .map_err(map_core_error)?;
        let project = engine.project().clone();
        state.emit(event);
        (project, existing)
    };

    state.persist_project_locked(&project).await;

    if let Some(asset) = removed_asset {
        let uploads_dir = state.inner.workspace_dir.join("uploads");
        if let Ok(canon_uploads) = tokio::fs::canonicalize(&uploads_dir).await {
            if !asset.path.starts_with("s3://") {
                if let Ok(canon_file) = tokio::fs::canonicalize(&asset.path).await
                    && canon_file.starts_with(&canon_uploads)
                {
                    let _ = tokio::fs::remove_file(canon_file).await;
                }
            } else {
                let expected_local = uploads_dir
                    .join(&id)
                    .join(format!("{}_{}", asset.id, asset.name));
                if let Ok(canon_file) = tokio::fs::canonicalize(&expected_local).await
                    && canon_file.starts_with(&canon_uploads)
                {
                    let _ = tokio::fs::remove_file(canon_file).await;
                }
            }
        }
        let proxies_dir = state.inner.workspace_dir.join("proxies");
        let proxy_file = proxies_dir.join(&id).join(format!("{media_id}.mp4"));
        if let Ok(canon_proxies) = tokio::fs::canonicalize(&proxies_dir).await
            && let Ok(canon_proxy) = tokio::fs::canonicalize(&proxy_file).await
            && canon_proxy.starts_with(&canon_proxies)
        {
            let _ = tokio::fs::remove_file(canon_proxy).await;
        }
    }

    Ok(Json(project))
}

#[derive(Deserialize)]
pub struct UpdateMediaRoleRequest {
    pub role_hint: String,
}

pub async fn update_media_role(
    State(state): State<AppState>,
    AxumPath((id, media_id)): AxumPath<(String, String)>,
    Json(req): Json<UpdateMediaRoleRequest>,
) -> Result<Json<solaris_core_ir::OpResult>, (StatusCode, String)> {
    validate_safe_id(&id)?;
    validate_safe_id(&media_id)?;

    let (result, project_snapshot) = {
        let mut guard = state.inner.projects.write().await;
        let engine = guard
            .get_mut(&id)
            .ok_or_else(|| (StatusCode::NOT_FOUND, format!("project '{id}' not found")))?;
        let (res, event) = engine
            .apply(
                TimelineCommand::UpdateMediaRole {
                    media_id,
                    role_hint: req.role_hint,
                },
                CommandMeta::default(),
            )
            .map_err(map_core_error)?;
        let snapshot = engine.project().clone();
        state.emit(event);
        (res, snapshot)
    };

    state.persist_project_locked(&project_snapshot).await;
    Ok(Json(result))
}

pub fn spawn_media_analysis_task(
    state_bg: AppState,
    project_id: String,
    asset: MediaAsset,
    path: PathBuf,
) {
    tokio::spawn(async move {
        let toolchain = state_bg.inner.toolchain.clone();
        let kind = asset.kind;
        let media_id = asset.id.clone();
        let path_fast = path.clone();

        let fast_intel = tokio::task::spawn_blocking(move || {
            probe_and_analyze_fast(&toolchain, &path_fast, kind)
        })
        .await
        .unwrap_or_default();

        // Merge fast probe fields into latest asset intelligence inside write lock so concurrent
        // transcripts or proxy paths are never overwritten by a stale snapshot.
        let snap1 = {
            let mut guard = state_bg.inner.projects.write().await;
            if let Some(engine) = guard.get_mut(&project_id)
                && let Some(existing) = engine
                    .project()
                    .media_pool
                    .iter()
                    .find(|m| m.id == media_id)
                    .map(|m| m.intelligence.clone())
            {
                let mut merged = fast_intel.clone();
                if merged.transcript.is_empty() && !existing.transcript.is_empty() {
                    merged.transcript = existing.transcript;
                }
                if merged.proxy_path.is_none() && existing.proxy_path.is_some() {
                    merged.proxy_path = existing.proxy_path;
                }
                if let Ok((_, op_event)) = engine.apply(
                    TimelineCommand::UpdateMediaIntelligence {
                        media_id: media_id.clone(),
                        intelligence: merged.clone(),
                    },
                    CommandMeta::default(),
                ) {
                    let project = engine.project().clone();
                    state_bg.emit(op_event);
                    state_bg.emit(EngineEvent::MediaAnalyzed {
                        project_id: project_id.clone(),
                        media_id: media_id.clone(),
                        intelligence: merged,
                    });
                    Some(project)
                } else {
                    None
                }
            } else {
                None
            }
        };
        if let Some(project) = snap1 {
            state_bg.persist_project_locked(&project).await;
        }

        if kind == MediaKind::Video && fast_intel.analyzed {
            let proxy_dir = state_bg
                .inner
                .workspace_dir
                .join("proxies")
                .join(&project_id);
            let proxy_path = proxy_dir.join(format!("{media_id}.mp4"));
            let proxy_path_str = proxy_path.to_string_lossy().to_string();
            let toolchain2 = state_bg.inner.toolchain.clone();
            let encoders = state_bg.inner.capabilities.hw_encoders.clone();
            let ok = tokio::task::spawn_blocking(move || {
                generate_proxy_with_encoders(&toolchain2, &path, &proxy_path, 720, &encoders)
                    .is_ok()
            })
            .await
            .unwrap_or(false);

            if ok {
                let snap2 = {
                    let mut guard = state_bg.inner.projects.write().await;
                    if let Some(engine) = guard.get_mut(&project_id)
                        && let Some(mut merged) = engine
                            .project()
                            .media_pool
                            .iter()
                            .find(|m| m.id == media_id)
                            .map(|m| m.intelligence.clone())
                    {
                        merged.proxy_path = Some(proxy_path_str);
                        if let Ok((_, op_event)) = engine.apply(
                            TimelineCommand::UpdateMediaIntelligence {
                                media_id: media_id.clone(),
                                intelligence: merged.clone(),
                            },
                            CommandMeta::default(),
                        ) {
                            let project = engine.project().clone();
                            state_bg.emit(op_event);
                            state_bg.emit(EngineEvent::MediaAnalyzed {
                                project_id,
                                media_id,
                                intelligence: merged,
                            });
                            Some(project)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };
                if let Some(project) = snap2 {
                    state_bg.persist_project_locked(&project).await;
                }
            }
        }
    });
}

pub async fn stream_media(
    State(state): State<AppState>,
    AxumPath((id, media_id)): AxumPath<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, (StatusCode, String)> {
    validate_safe_id(&id)?;
    let (proxy_opt, orig_path) = {
        let guard = state.inner.projects.read().await;
        let engine = guard
            .get(&id)
            .ok_or_else(|| (StatusCode::NOT_FOUND, format!("project '{id}' not found")))?;
        let asset = engine
            .project()
            .media_pool
            .iter()
            .find(|m| m.id == media_id)
            .ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    format!("media '{media_id}' not found"),
                )
            })?;
        (asset.intelligence.proxy_path.clone(), asset.path.clone())
    };

    let safe_path = match proxy_opt {
        Some(proxy_str) => match state.resolve_readable_path_async(proxy_str).await {
            Ok(p) => p,
            Err(_) => state
                .resolve_readable_path_async(orig_path)
                .await
                .map_err(map_storage_error)?,
        },
        None => state
            .resolve_readable_path_async(orig_path)
            .await
            .map_err(map_storage_error)?,
    };

    stream_file_with_range(safe_path, headers, None).await
}

enum ByteRangeParse {
    Satisfiable(u64, u64),
    Unsatisfiable,
    InvalidSyntax,
}

pub async fn stream_file_with_range(
    safe_path: PathBuf,
    headers: HeaderMap,
    attachment_name: Option<&str>,
) -> Result<Response, (StatusCode, String)> {
    let mut file = File::open(&safe_path)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let meta = file
        .metadata()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if !meta.is_file() {
        return Err((
            StatusCode::NOT_FOUND,
            "requested media path is not a regular file".to_string(),
        ));
    }
    let total_len = meta.len();
    let mime = guess_mime(&safe_path);

    if let Some(range_header) = headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
        match parse_byte_range(range_header, total_len) {
            ByteRangeParse::Satisfiable(start, end) => {
                let len = end - start + 1;
                file.seek(std::io::SeekFrom::Start(start))
                    .await
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

                let stream = ReaderStream::new(file.take(len));
                let mut resp = Response::new(Body::from_stream(stream));
                *resp.status_mut() = StatusCode::PARTIAL_CONTENT;
                resp.headers_mut().insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_str(mime)
                        .unwrap_or(HeaderValue::from_static("application/octet-stream")),
                );
                resp.headers_mut().insert(
                    header::X_CONTENT_TYPE_OPTIONS,
                    HeaderValue::from_static("nosniff"),
                );
                resp.headers_mut().insert(
                    header::CONTENT_SECURITY_POLICY,
                    HeaderValue::from_static("default-src 'none'; sandbox"),
                );
                resp.headers_mut()
                    .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
                if let Ok(len_val) = HeaderValue::from_str(&len.to_string()) {
                    resp.headers_mut().insert(header::CONTENT_LENGTH, len_val);
                }
                resp.headers_mut().insert(
                    header::CONTENT_RANGE,
                    HeaderValue::from_str(&format!("bytes {start}-{end}/{total_len}"))
                        .unwrap_or(HeaderValue::from_static("bytes */0")),
                );
                if let Some(name) = attachment_name {
                    let safe_disp = sanitize_file_name(name);
                    if let Ok(val) =
                        HeaderValue::from_str(&format!("attachment; filename=\"{safe_disp}\""))
                    {
                        resp.headers_mut().insert(header::CONTENT_DISPOSITION, val);
                    }
                }
                return Ok(resp);
            }
            ByteRangeParse::Unsatisfiable => {
                let mut resp = Response::new(Body::empty());
                *resp.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;
                if let Ok(val) = HeaderValue::from_str(&format!("bytes */{total_len}")) {
                    resp.headers_mut().insert(header::CONTENT_RANGE, val);
                }
                return Ok(resp);
            }
            // Per RFC 9110 §14.2: ignore syntactically invalid Range headers and return 200 OK
            ByteRangeParse::InvalidSyntax => {}
        }
    }

    let stream = ReaderStream::new(file);
    let mut resp = Response::new(Body::from_stream(stream));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(mime).unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    resp.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    resp.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    resp.headers_mut()
        .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    if let Ok(len_val) = HeaderValue::from_str(&total_len.to_string()) {
        resp.headers_mut().insert(header::CONTENT_LENGTH, len_val);
    }
    if let Some(name) = attachment_name {
        let safe_disp = sanitize_file_name(name);
        if let Ok(val) = HeaderValue::from_str(&format!("attachment; filename=\"{safe_disp}\"")) {
            resp.headers_mut().insert(header::CONTENT_DISPOSITION, val);
        }
    }
    Ok(resp)
}

pub fn sanitize_file_name(raw: &str) -> String {
    let mut sanitized = String::with_capacity(raw.len().min(128));
    for ch in raw.chars() {
        if sanitized.len() >= 128 {
            break;
        }
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
        }
    }
    while sanitized.contains("..") {
        sanitized = sanitized.replace("..", "_");
    }
    let trimmed = sanitized.trim_matches('.').trim_matches('_');
    if trimmed.is_empty() {
        "upload.bin".to_string()
    } else {
        trimmed.to_string()
    }
}

fn infer_media_kind(name: &str) -> MediaKind {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "mp4" | "mov" | "webm" | "mkv" | "m4v" => MediaKind::Video,
        "png" | "jpg" | "jpeg" | "webp" | "avif" | "gif" | "svg" => MediaKind::Image,
        "wav" | "mp3" | "aac" | "m4a" | "ogg" | "flac" => MediaKind::Audio,
        _ => MediaKind::Document,
    }
}

fn infer_default_role_hint(kind: MediaKind, name: &str) -> &'static str {
    match kind {
        MediaKind::Video => "a_roll",
        MediaKind::Image => "b_roll",
        MediaKind::Audio => {
            let lower = name.to_ascii_lowercase();
            if lower.contains("voice")
                || lower.contains("vo_")
                || lower.contains("_vo")
                || lower.contains("narration")
                || lower.contains("speech")
                || lower.contains("mic")
                || lower.contains("dialog")
            {
                "voice"
            } else if lower.contains("sfx")
                || lower.contains("whoosh")
                || lower.contains("riser")
                || lower.contains("hit")
                || lower.contains("boom")
            {
                "sfx"
            } else {
                "music"
            }
        }
        MediaKind::Document => "reference",
    }
}

fn guess_mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "pdf" => "application/pdf",
        "md" | "txt" | "cube" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn parse_byte_range(header: &str, total_len: u64) -> ByteRangeParse {
    let Some(spec) = header.trim().strip_prefix("bytes=") else {
        return ByteRangeParse::InvalidSyntax;
    };
    // Multi-range requests are ignored (return full 200 OK) rather than 416
    if spec.contains(',') {
        return ByteRangeParse::InvalidSyntax;
    }
    let Some((start_s, end_s)) = spec.split_once('-') else {
        return ByteRangeParse::InvalidSyntax;
    };
    let start_s = start_s.trim();
    let end_s = end_s.trim();
    if start_s.is_empty() {
        let Ok(suffix) = end_s.parse::<u64>() else {
            return ByteRangeParse::InvalidSyntax;
        };
        if suffix == 0 || total_len == 0 {
            return ByteRangeParse::Unsatisfiable;
        }
        let start = total_len.saturating_sub(suffix);
        return ByteRangeParse::Satisfiable(start, total_len - 1);
    }
    let Ok(start) = start_s.parse::<u64>() else {
        return ByteRangeParse::InvalidSyntax;
    };
    let end_opt = if end_s.is_empty() {
        None
    } else {
        let Ok(v) = end_s.parse::<u64>() else {
            return ByteRangeParse::InvalidSyntax;
        };
        if v < start {
            return ByteRangeParse::InvalidSyntax;
        }
        Some(v)
    };
    if total_len == 0 || start >= total_len {
        return ByteRangeParse::Unsatisfiable;
    }
    let end = end_opt
        .map(|v| v.min(total_len - 1))
        .unwrap_or(total_len - 1);
    ByteRangeParse::Satisfiable(start, end)
}
