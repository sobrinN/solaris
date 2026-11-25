use std::collections::HashSet;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use solaris_agent_runtime::lint_gfx_html;
use solaris_core_ir::{
    CodeAssetPackManifest, SavedCodeAsset, build_asset_pack_manifest, validate_asset_pack_manifest,
    validate_single_asset,
};

use crate::state::{AppState, save_json_atomic_async, validate_safe_id};

pub async fn list_assets(State(state): State<AppState>) -> Json<Vec<SavedCodeAsset>> {
    let guard = state.inner.assets.read().await;
    let mut list: Vec<SavedCodeAsset> = guard.values().cloned().collect();
    list.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    Json(list)
}

pub async fn save_asset(
    State(state): State<AppState>,
    Json(mut asset): Json<SavedCodeAsset>,
) -> Result<Json<SavedCodeAsset>, (StatusCode, String)> {
    if asset.id.is_empty() {
        asset.id = Uuid::new_v4().to_string();
    } else {
        validate_safe_id(&asset.id)?;
    }
    validate_single_asset(&asset).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let lint = lint_gfx_html(&asset.html_source);
    if !lint.valid {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("GFX HTML failed lint: {}", lint.errors.join("; ")),
        ));
    }
    asset.created_at = Utc::now();
    let path = state
        .inner
        .workspace_dir
        .join("assets")
        .join(format!("{}.json", asset.id));
    let _persist = state.inner.asset_persist_lock.lock().await;
    save_json_atomic_async(asset.clone(), path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    state
        .undismiss_starter_locked(&asset.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    state
        .inner
        .assets
        .write()
        .await
        .insert(asset.id.clone(), asset.clone());
    drop(_persist);
    state.emit_assets_changed("saved", Some(&asset.id));
    Ok(Json(asset))
}

#[derive(Serialize)]
pub struct AssetDeleteResponse {
    pub deleted: bool,
    pub asset_id: String,
}

pub async fn delete_asset(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<AssetDeleteResponse>, (StatusCode, String)> {
    state.delete_asset(&id).await?;
    Ok(Json(AssetDeleteResponse {
        deleted: true,
        asset_id: id,
    }))
}

#[derive(Serialize)]
pub struct AssetRestoreResponse {
    pub restored_count: usize,
    pub assets: Vec<SavedCodeAsset>,
}

pub async fn restore_starter_assets(
    State(state): State<AppState>,
) -> Result<Json<AssetRestoreResponse>, (StatusCode, String)> {
    let (restored_count, assets) = state.restore_starter_assets().await?;
    Ok(Json(AssetRestoreResponse {
        restored_count,
        assets,
    }))
}

#[derive(Deserialize, Default)]
pub struct ExportPackQuery {
    #[serde(default)]
    pub ids: Option<String>,
}

pub async fn export_asset_pack(
    State(state): State<AppState>,
    Query(query): Query<ExportPackQuery>,
) -> Result<Json<CodeAssetPackManifest>, (StatusCode, String)> {
    let filter: Option<HashSet<String>> = query
        .ids
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect::<HashSet<_>>()
        })
        .filter(|set| !set.is_empty());

    let guard = state.inner.assets.read().await;
    let mut list: Vec<SavedCodeAsset> = guard
        .values()
        .filter(|a| filter.as_ref().is_none_or(|set| set.contains(&a.id)))
        .cloned()
        .collect();
    list.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));

    if filter.is_some() && list.is_empty() {
        return Err((
            StatusCode::NOT_FOUND,
            "no matching assets found for requested ids".to_string(),
        ));
    }

    Ok(Json(build_asset_pack_manifest(
        "solaris-workspace-pack",
        "Solaris Workspace Asset Pack",
        list,
    )))
}

#[derive(Serialize)]
pub struct AssetPackImportResponse {
    pub imported_count: usize,
    pub assets: Vec<SavedCodeAsset>,
}

pub async fn import_asset_pack(
    State(state): State<AppState>,
    Json(manifest): Json<CodeAssetPackManifest>,
) -> Result<Json<AssetPackImportResponse>, (StatusCode, String)> {
    if let Err(errs) = validate_asset_pack_manifest(&manifest) {
        return Err((StatusCode::BAD_REQUEST, errs.join("; ")));
    }

    let mut validated_assets = Vec::with_capacity(manifest.assets.len());
    for mut asset in manifest.assets {
        validate_safe_id(&asset.id)?;
        let lint = lint_gfx_html(&asset.html_source);
        if !lint.valid {
            return Err((
                StatusCode::BAD_REQUEST,
                format!(
                    "asset '{}' failed GFX HTML lint: {}",
                    asset.id,
                    lint.errors.join("; ")
                ),
            ));
        }
        asset.created_at = Utc::now();
        validated_assets.push(asset);
    }

    let _persist = state.inner.asset_persist_lock.lock().await;
    for asset in &validated_assets {
        let path = state
            .inner
            .workspace_dir
            .join("assets")
            .join(format!("{}.json", asset.id));
        save_json_atomic_async(asset.clone(), path)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
        state
            .undismiss_starter_locked(&asset.id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    }
    {
        let mut guard = state.inner.assets.write().await;
        for asset in &validated_assets {
            guard.insert(asset.id.clone(), asset.clone());
        }
    }
    drop(_persist);
    state.emit_assets_changed("imported", None);

    Ok(Json(AssetPackImportResponse {
        imported_count: validated_assets.len(),
        assets: validated_assets,
    }))
}
