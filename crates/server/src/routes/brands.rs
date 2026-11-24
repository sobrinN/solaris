use std::path::PathBuf;

use axum::Json;
use axum::extract::{Multipart, Path as AxumPath, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use solaris_core_ir::{
    BrandPlaybook, BrandReferenceAsset, BrandReferenceKind, MediaIntelligenceManifest, MediaKind,
};
use solaris_media_engine::{ParsedCubeLut, parse_cube_file, probe_and_analyze_fast};

use crate::routes::media::{
    LocalPathItem, sanitize_file_name, stream_file_with_range, stream_multipart_field_to_workspace,
};
use crate::state::{AppState, map_storage_error, save_json_atomic_async, validate_safe_id};

const MAX_BRAND_JSON_BYTES: usize = 2 * 1024 * 1024;

pub async fn list_brands(State(state): State<AppState>) -> Json<Vec<BrandPlaybook>> {
    let guard = state.inner.brands.read().await;
    let mut list: Vec<BrandPlaybook> = guard.values().cloned().collect();
    list.sort_by(|a, b| {
        let a_def = a.id == "solaris-default";
        let b_def = b.id == "solaris-default";
        b_def
            .cmp(&a_def)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    Json(list)
}

pub async fn upsert_brand(
    State(state): State<AppState>,
    Json(mut brand): Json<BrandPlaybook>,
) -> Result<Json<BrandPlaybook>, (StatusCode, String)> {
    validate_safe_id(&brand.id)?;
    brand.updated_at = Utc::now();
    let path = state
        .inner
        .workspace_dir
        .join("brands")
        .join(format!("{}.json", brand.id));
    let _persist = state.inner.brand_persist_lock.lock().await;
    save_json_atomic_async(brand.clone(), path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    state
        .inner
        .brands
        .write()
        .await
        .insert(brand.id.clone(), brand.clone());
    Ok(Json(brand))
}

pub async fn update_brand(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(mut brand): Json<BrandPlaybook>,
) -> Result<Json<BrandPlaybook>, (StatusCode, String)> {
    validate_safe_id(&id)?;
    brand.id = id.clone();
    brand.updated_at = Utc::now();

    let _persist = state.inner.brand_persist_lock.lock().await;
    {
        let guard = state.inner.brands.read().await;
        if let Some(existing) = guard.get(&id) {
            // Preserve server-side background analysis on reference_media items
            for item in &mut brand.reference_media {
                if !item.intelligence.analyzed
                    && let Some(prev) = existing.reference_media.iter().find(|r| r.id == item.id)
                    && prev.intelligence.analyzed
                {
                    item.intelligence = prev.intelligence.clone();
                }
            }
        }
    }

    let path = state
        .inner
        .workspace_dir
        .join("brands")
        .join(format!("{}.json", brand.id));
    save_json_atomic_async(brand.clone(), path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    state
        .inner
        .brands
        .write()
        .await
        .insert(brand.id.clone(), brand.clone());
    Ok(Json(brand))
}

pub async fn upload_brand_media(
    State(state): State<AppState>,
    AxumPath(brand_id): AxumPath<String>,
    req: Request,
) -> Result<Json<BrandPlaybook>, (StatusCode, String)> {
    validate_safe_id(&brand_id)?;
    {
        let guard = state.inner.brands.read().await;
        if !guard.contains_key(&brand_id) {
            return Err((
                StatusCode::NOT_FOUND,
                format!("brand '{brand_id}' not found"),
            ));
        }
    }

    let content_type = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();

    let mut added: Vec<(BrandReferenceAsset, PathBuf)> = Vec::new();

    if content_type.starts_with("multipart/form-data") {
        let mut multipart =
            <Multipart as axum::extract::FromRequest<AppState>>::from_request(req, &state)
                .await
                .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

        let mut pending_notes = String::new();
        while let Some(field) = multipart
            .next_field()
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
        {
            if field.name() == Some("notes") {
                pending_notes = field.text().await.unwrap_or_default();
                continue;
            }
            let file_name = field
                .file_name()
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("ref-{}.bin", Uuid::new_v4()));
            let asset_id = Uuid::new_v4().to_string();
            let safe_name = sanitize_file_name(&file_name);
            let rel_key = format!("brands_media/{brand_id}/{asset_id}_{safe_name}");
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

            let kind = infer_brand_reference_kind(&safe_name);
            let asset = BrandReferenceAsset {
                id: asset_id,
                name: safe_name,
                kind,
                path: synced_path,
                notes: String::new(),
                added_at: Utc::now(),
                intelligence: MediaIntelligenceManifest::default(),
            };
            added.push((asset, saved_path));
        }
        if !pending_notes.is_empty() {
            for (asset, _) in &mut added {
                asset.notes = pending_notes.clone();
            }
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
        #[derive(Deserialize)]
        struct BrandPathImport {
            paths: Vec<LocalPathItem>,
            #[serde(default)]
            notes: String,
        }
        let bytes = axum::body::to_bytes(req.into_body(), MAX_BRAND_JSON_BYTES)
            .await
            .map_err(|e| (StatusCode::PAYLOAD_TOO_LARGE, e.to_string()))?;
        let body: BrandPathImport =
            serde_json::from_slice(&bytes).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

        for item in body.paths {
            let resolved = state
                .resolve_readable_path_async(item.path)
                .await
                .map_err(map_storage_error)?;
            let name = resolved
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("reference")
                .to_string();
            let kind = infer_brand_reference_kind(&name);
            let asset = BrandReferenceAsset {
                id: Uuid::new_v4().to_string(),
                name,
                kind,
                path: resolved.to_string_lossy().to_string(),
                notes: body.notes.clone(),
                added_at: Utc::now(),
                intelligence: MediaIntelligenceManifest::default(),
            };
            added.push((asset, resolved));
        }
    }

    let updated_brand = {
        let _persist = state.inner.brand_persist_lock.lock().await;
        let mut next = {
            let guard = state.inner.brands.read().await;
            guard.get(&brand_id).cloned().ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    format!("brand '{brand_id}' not found"),
                )
            })?
        };
        for (asset, _) in &added {
            if asset.kind == BrandReferenceKind::LutCube {
                next.default_lut = Some(asset.path.clone());
            }
            next.reference_media.push(asset.clone());
        }
        next.updated_at = Utc::now();
        let path = state
            .inner
            .workspace_dir
            .join("brands")
            .join(format!("{}.json", next.id));
        save_json_atomic_async(next.clone(), path)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
        state
            .inner
            .brands
            .write()
            .await
            .insert(brand_id.clone(), next.clone());
        next
    };

    for (asset, path) in added {
        let media_kind = match asset.kind {
            BrandReferenceKind::ReferenceVideo => Some(MediaKind::Video),
            BrandReferenceKind::AudioIdentity => Some(MediaKind::Audio),
            _ => None,
        };
        if let Some(mk) = media_kind {
            let state_bg = state.clone();
            let b_id = brand_id.clone();
            let a_id = asset.id.clone();
            tokio::spawn(async move {
                let toolchain = state_bg.inner.toolchain.clone();
                let intel = tokio::task::spawn_blocking(move || {
                    probe_and_analyze_fast(&toolchain, &path, mk)
                })
                .await
                .unwrap_or_default();
                let _persist = state_bg.inner.brand_persist_lock.lock().await;
                let next_snapshot = {
                    let guard = state_bg.inner.brands.read().await;
                    guard.get(&b_id).cloned().and_then(|mut brand| {
                        let item = brand.reference_media.iter_mut().find(|r| r.id == a_id)?;
                        item.intelligence = intel;
                        brand.updated_at = Utc::now();
                        Some(brand)
                    })
                };
                if let Some(snapshot) = next_snapshot {
                    let json_path = state_bg
                        .inner
                        .workspace_dir
                        .join("brands")
                        .join(format!("{}.json", snapshot.id));
                    if save_json_atomic_async(snapshot.clone(), json_path)
                        .await
                        .is_ok()
                    {
                        state_bg.inner.brands.write().await.insert(b_id, snapshot);
                    }
                }
            });
        }
    }

    Ok(Json(updated_brand))
}

#[derive(Deserialize)]
pub struct UpdateBrandMediaRequest {
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub kind: Option<BrandReferenceKind>,
}

pub async fn update_brand_media(
    State(state): State<AppState>,
    AxumPath((brand_id, asset_id)): AxumPath<(String, String)>,
    Json(req): Json<UpdateBrandMediaRequest>,
) -> Result<Json<BrandPlaybook>, (StatusCode, String)> {
    validate_safe_id(&brand_id)?;
    let _persist = state.inner.brand_persist_lock.lock().await;
    let mut next = {
        let guard = state.inner.brands.read().await;
        guard.get(&brand_id).cloned().ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("brand '{brand_id}' not found"),
            )
        })?
    };
    let mut changed_path: Option<String> = None;
    let mut new_kind: Option<BrandReferenceKind> = None;
    {
        let item = next
            .reference_media
            .iter_mut()
            .find(|r| r.id == asset_id)
            .ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    format!("brand reference '{asset_id}' not found"),
                )
            })?;
        if let Some(notes) = req.notes {
            item.notes = notes;
        }
        if let Some(kind) = req.kind {
            item.kind = kind;
            changed_path = Some(item.path.clone());
            new_kind = Some(kind);
        }
    }

    // Keep brand.default_lut synchronized when a reference's kind switches to/from LutCube
    if let (Some(p), Some(k)) = (changed_path, new_kind) {
        if k == BrandReferenceKind::LutCube {
            next.default_lut = Some(p);
        } else if next.default_lut.as_deref() == Some(p.as_str()) {
            next.default_lut = next
                .reference_media
                .iter()
                .find(|r| r.kind == BrandReferenceKind::LutCube)
                .map(|r| r.path.clone());
        }
    }

    next.updated_at = Utc::now();
    let path = state
        .inner
        .workspace_dir
        .join("brands")
        .join(format!("{}.json", next.id));
    save_json_atomic_async(next.clone(), path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    state
        .inner
        .brands
        .write()
        .await
        .insert(brand_id, next.clone());
    Ok(Json(next))
}

pub async fn delete_brand_media(
    State(state): State<AppState>,
    AxumPath((brand_id, asset_id)): AxumPath<(String, String)>,
) -> Result<Json<BrandPlaybook>, (StatusCode, String)> {
    validate_safe_id(&brand_id)?;
    let (next_brand, removed) = {
        let _persist = state.inner.brand_persist_lock.lock().await;
        let mut next = {
            let guard = state.inner.brands.read().await;
            guard.get(&brand_id).cloned().ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    format!("brand '{brand_id}' not found"),
                )
            })?
        };
        let Some(pos) = next.reference_media.iter().position(|r| r.id == asset_id) else {
            return Err((
                StatusCode::NOT_FOUND,
                format!("brand reference '{asset_id}' not found"),
            ));
        };
        let removed = next.reference_media.remove(pos);

        if next.default_lut.as_deref() == Some(removed.path.as_str()) {
            next.default_lut = next
                .reference_media
                .iter()
                .find(|r| r.kind == BrandReferenceKind::LutCube)
                .map(|r| r.path.clone());
        }

        next.updated_at = Utc::now();
        let path = state
            .inner
            .workspace_dir
            .join("brands")
            .join(format!("{}.json", next.id));
        save_json_atomic_async(next.clone(), path)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
        state
            .inner
            .brands
            .write()
            .await
            .insert(brand_id.clone(), next.clone());
        (next, removed)
    };

    // Scope local file deletion strictly to this brand's own brands_media/{brand_id}/ directory
    // (works cleanly for both LocalFsStorage paths and S3 cached/uploaded files)
    let brand_dir = state
        .inner
        .workspace_dir
        .join("brands_media")
        .join(&brand_id);
    if let Ok(canon_brand_dir) = tokio::fs::canonicalize(&brand_dir).await {
        if !removed.path.starts_with("s3://") {
            if let Ok(canon_file) = tokio::fs::canonicalize(&removed.path).await
                && canon_file.starts_with(&canon_brand_dir)
            {
                let _ = tokio::fs::remove_file(canon_file).await;
            }
        } else {
            // Also clean up any matching local upload copy under brands_media/{brand_id}/
            let expected_local = brand_dir.join(format!("{}_{}", removed.id, removed.name));
            if let Ok(canon_file) = tokio::fs::canonicalize(&expected_local).await
                && canon_file.starts_with(&canon_brand_dir)
            {
                let _ = tokio::fs::remove_file(canon_file).await;
            }
        }
    }

    Ok(Json(next_brand))
}

pub async fn stream_brand_media(
    State(state): State<AppState>,
    AxumPath((brand_id, asset_id)): AxumPath<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, (StatusCode, String)> {
    validate_safe_id(&brand_id)?;
    let path_str = {
        let guard = state.inner.brands.read().await;
        let brand = guard.get(&brand_id).ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("brand '{brand_id}' not found"),
            )
        })?;
        let item = brand
            .reference_media
            .iter()
            .find(|r| r.id == asset_id)
            .ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    format!("brand reference '{asset_id}' not found"),
                )
            })?;
        item.path.clone()
    };
    let safe_path = state
        .resolve_readable_path_async(path_str)
        .await
        .map_err(map_storage_error)?;
    stream_file_with_range(safe_path, headers, None).await
}

pub async fn get_brand_lut(
    State(state): State<AppState>,
    AxumPath(brand_id): AxumPath<String>,
) -> Result<Json<ParsedCubeLut>, (StatusCode, String)> {
    validate_safe_id(&brand_id)?;
    let lut_path = {
        let guard = state.inner.brands.read().await;
        let brand = guard.get(&brand_id).ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("brand '{brand_id}' not found"),
            )
        })?;
        brand.default_lut.clone().ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "no default_lut configured for this brand".to_string(),
            )
        })?
    };
    let safe_path = state
        .resolve_readable_path_async(lut_path)
        .await
        .map_err(map_storage_error)?;
    let parsed = tokio::task::spawn_blocking(move || parse_cube_file(&safe_path))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(parsed))
}

fn infer_brand_reference_kind(name: &str) -> BrandReferenceKind {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "mp4" | "mov" | "webm" | "mkv" | "m4v" => BrandReferenceKind::ReferenceVideo,
        "cube" => BrandReferenceKind::LutCube,
        "woff2" | "woff" | "ttf" | "otf" => BrandReferenceKind::Font,
        "wav" | "mp3" | "aac" | "m4a" | "ogg" | "flac" => BrandReferenceKind::AudioIdentity,
        "svg" => BrandReferenceKind::Logo,
        "png" | "jpg" | "jpeg" | "webp" | "avif" => {
            if name.to_lowercase().contains("logo") {
                BrandReferenceKind::Logo
            } else {
                BrandReferenceKind::StyleFrame
            }
        }
        _ => BrandReferenceKind::Document,
    }
}
