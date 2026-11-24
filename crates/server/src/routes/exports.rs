use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use chrono::Utc;
use uuid::Uuid;

use solaris_core_ir::{
    ClipKind, EngineEvent, ExportFormat, ExportJobStatus, ExportRequest, ExportState,
    SavedCodeAsset, TrackKind,
};
use solaris_media_engine::{
    ExportPlanOptions, execute_export_with_progress,
    render_project_overlays_via_chromium_with_assets,
};

use crate::routes::media::{sanitize_file_name, stream_file_with_range};
use crate::state::{AppState, validate_safe_id};

pub async fn start_export(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(req): Json<ExportRequest>,
) -> Result<(StatusCode, Json<ExportJobStatus>), (StatusCode, String)> {
    let initial = enqueue_export_job(&state, &id, req).await?;
    Ok((StatusCode::ACCEPTED, Json(initial)))
}

pub async fn enqueue_export_job(
    state: &AppState,
    id: &str,
    req: ExportRequest,
) -> Result<ExportJobStatus, (StatusCode, String)> {
    let id = id.to_string();
    validate_safe_id(&id)?;
    let mut project = {
        let guard = state.inner.projects.read().await;
        let engine = guard
            .get(&id)
            .ok_or_else(|| (StatusCode::NOT_FOUND, format!("project '{id}' not found")))?;
        engine.project().clone()
    };

    let (brand_tokens, brand_default_lut, brand_fonts) = state
        .load_brand_render_context(&project.brand_playbook_id)
        .await;

    let workspace_assets: Vec<SavedCodeAsset> =
        state.inner.assets.read().await.values().cloned().collect();

    // Resolve all referenced media_pool paths and clip-level LUT paths through AssetStorage
    // so S3 objects are cached locally and external/SSRF paths are blocked before FFmpeg runs.
    let mut referenced_media_ids: HashSet<String> = HashSet::new();
    for track in &project.tracks {
        if track.muted {
            continue;
        }
        for clip in &track.clips {
            if let ClipKind::Media { media_id, .. } = &clip.payload {
                referenced_media_ids.insert(media_id.clone());
            }
        }
    }
    let mut resolved_media_paths: HashMap<String, String> = HashMap::new();
    for asset in &project.media_pool {
        if referenced_media_ids.contains(&asset.id) {
            if asset.path.trim().is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!(
                        "referenced media '{}' has no readable file path",
                        asset.name
                    ),
                ));
            }
            let safe = state
                .resolve_readable_path_async(asset.path.clone())
                .await
                .map_err(|e| (StatusCode::FORBIDDEN, e.to_string()))?;
            resolved_media_paths.insert(asset.id.clone(), safe.to_string_lossy().to_string());
        }
    }
    for asset in &mut project.media_pool {
        if let Some(local_path) = resolved_media_paths.get(&asset.id) {
            asset.path = local_path.clone();
        }
    }
    for track in &mut project.tracks {
        if track.muted {
            continue;
        }
        for clip in &mut track.clips {
            if let ClipKind::Grade { grade } = &mut clip.payload
                && let Some(raw_clip_lut) = grade.lut_path.clone().filter(|s| !s.trim().is_empty())
            {
                let safe_lut = state
                    .resolve_readable_path_async(raw_clip_lut)
                    .await
                    .map_err(|e| (StatusCode::FORBIDDEN, e.to_string()))?;
                grade.lut_path = Some(safe_lut.to_string_lossy().to_string());
            }
        }
    }

    let raw_lut = project.active_grade.lut_path.clone().or(brand_default_lut);
    let resolved_lut_path: Option<PathBuf> = match raw_lut.filter(|s| !s.trim().is_empty()) {
        Some(p) => Some(
            state
                .resolve_readable_path_async(p)
                .await
                .map_err(|e| (StatusCode::FORBIDDEN, e.to_string()))?,
        ),
        None => None,
    };

    let job_id = Uuid::new_v4().to_string();
    let short_job = &job_id[..8];
    let ext = match req.format {
        ExportFormat::Mp4 => "mp4",
        ExportFormat::MovProRes4444 => "mov",
        ExportFormat::WebmAlpha => "webm",
    };
    let base_name = req
        .output_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&project.name);
    let safe_base = sanitize_file_name(base_name);
    let output_path = state
        .inner
        .workspace_dir
        .join("exports")
        .join(format!("{safe_base}_{short_job}.{ext}"));

    let initial = ExportJobStatus {
        id: job_id.clone(),
        project_id: id.clone(),
        state: ExportState::Queued,
        progress: 0.05,
        stage_label: "Queued for dual-path render".to_string(),
        output_path: None,
        error: None,
        updated_at: Utc::now(),
    };
    {
        let mut jobs = state.inner.export_jobs.write().await;
        if jobs.len() >= 64
            && let Some(oldest_id) = jobs
                .iter()
                .filter(|(_, j)| matches!(j.state, ExportState::Completed | ExportState::Failed))
                .min_by_key(|(_, j)| j.updated_at)
                .map(|(k, _)| k.clone())
        {
            jobs.remove(&oldest_id);
        }
        jobs.insert(job_id.clone(), initial.clone());
    }
    state.emit(EngineEvent::ExportProgress {
        project_id: id.clone(),
        job: initial.clone(),
    });

    let state_bg = state.clone();
    let job_id_bg = job_id.clone();
    tokio::spawn(async move {
        let Ok(_export_permit) = state_bg
            .inner
            .export_semaphore
            .clone()
            .acquire_owned()
            .await
        else {
            return;
        };

        let has_overlays = project.tracks.iter().any(|t| {
            !t.muted && matches!(t.kind, TrackKind::Gfx | TrackKind::Text) && !t.clips.is_empty()
        });

        let overlay_tmp_dir = state_bg
            .inner
            .workspace_dir
            .join("exports")
            .join(format!(".tmp_{job_id_bg}"));

        let overlay_images = if has_overlays {
            if state_bg.inner.toolchain.chromium_path.is_none() {
                let failed = ExportJobStatus {
                    id: job_id_bg.clone(),
                    project_id: id.clone(),
                    state: ExportState::Failed,
                    progress: 0.1,
                    stage_label: "Export failed: headless Chromium unavailable".to_string(),
                    output_path: None,
                    error: Some(
                        "Project contains active GFX/Caption overlays, but headless Chromium is not installed (set SOLARIS_CHROME_PATH or install chromium)."
                            .to_string(),
                    ),
                    updated_at: Utc::now(),
                };
                state_bg
                    .inner
                    .export_jobs
                    .write()
                    .await
                    .insert(job_id_bg, failed.clone());
                state_bg.emit(EngineEvent::ExportProgress {
                    project_id: id,
                    job: failed,
                });
                return;
            }

            let rendering_gfx = ExportJobStatus {
                id: job_id_bg.clone(),
                project_id: id.clone(),
                state: ExportState::RenderingGfx,
                progress: 0.15,
                stage_label: "Rendering GFX & caption overlays via Chromium".to_string(),
                output_path: None,
                error: None,
                updated_at: Utc::now(),
            };
            state_bg
                .inner
                .export_jobs
                .write()
                .await
                .insert(job_id_bg.clone(), rendering_gfx.clone());
            state_bg.emit(EngineEvent::ExportProgress {
                project_id: id.clone(),
                job: rendering_gfx,
            });

            let tc = state_bg.inner.toolchain.clone();
            let proj_clone = project.clone();
            let tokens_clone = brand_tokens.clone();
            let assets_clone = workspace_assets.clone();
            let fonts_clone = brand_fonts.clone();
            let tmp_clone = overlay_tmp_dir.clone();
            let render_res = tokio::task::spawn_blocking(move || {
                render_project_overlays_via_chromium_with_assets(
                    &tc,
                    &proj_clone,
                    &tokens_clone,
                    &assets_clone,
                    &fonts_clone,
                    &tmp_clone,
                )
            })
            .await;

            match render_res {
                Ok(Ok(specs)) => specs,
                Ok(Err(err)) => {
                    let _ = tokio::fs::remove_dir_all(&overlay_tmp_dir).await;
                    let failed = ExportJobStatus {
                        id: job_id_bg.clone(),
                        project_id: id.clone(),
                        state: ExportState::Failed,
                        progress: 0.15,
                        stage_label: "Export failed during overlay rendering".to_string(),
                        output_path: None,
                        error: Some(format!("Overlay render failed: {err}")),
                        updated_at: Utc::now(),
                    };
                    state_bg
                        .inner
                        .export_jobs
                        .write()
                        .await
                        .insert(job_id_bg, failed.clone());
                    state_bg.emit(EngineEvent::ExportProgress {
                        project_id: id,
                        job: failed,
                    });
                    return;
                }
                Err(join_err) => {
                    let _ = tokio::fs::remove_dir_all(&overlay_tmp_dir).await;
                    let failed = ExportJobStatus {
                        id: job_id_bg.clone(),
                        project_id: id.clone(),
                        state: ExportState::Failed,
                        progress: 0.15,
                        stage_label: "Export overlay render task panicked".to_string(),
                        output_path: None,
                        error: Some(format!("Overlay render task panicked: {join_err}")),
                        updated_at: Utc::now(),
                    };
                    state_bg
                        .inner
                        .export_jobs
                        .write()
                        .await
                        .insert(job_id_bg, failed.clone());
                    state_bg.emit(EngineEvent::ExportProgress {
                        project_id: id,
                        job: failed,
                    });
                    return;
                }
            }
        } else {
            Vec::new()
        };

        let mut compositing = ExportJobStatus {
            id: job_id_bg.clone(),
            project_id: id.clone(),
            state: ExportState::Compositing,
            progress: 0.25,
            stage_label: "Compositing timeline & -14 LUFS audio master".to_string(),
            output_path: None,
            error: None,
            updated_at: Utc::now(),
        };
        state_bg
            .inner
            .export_jobs
            .write()
            .await
            .insert(job_id_bg.clone(), compositing.clone());
        state_bg.emit(EngineEvent::ExportProgress {
            project_id: id.clone(),
            job: compositing.clone(),
        });

        let toolchain = state_bg.inner.toolchain.clone();
        let hw_encoders = state_bg.inner.capabilities.hw_encoders.clone();
        let out_clone = output_path.clone();
        let state_prog = state_bg.clone();
        let job_prog = job_id_bg.clone();
        let proj_id_prog = id.clone();

        let res = tokio::task::spawn_blocking(move || {
            let mut last_emit = Instant::now();
            execute_export_with_progress(
                &toolchain,
                &project,
                &req,
                &out_clone,
                ExportPlanOptions {
                    overlay_sequence_dir: None,
                    overlay_images: &overlay_images,
                    resolved_lut_path: resolved_lut_path.as_deref(),
                },
                &hw_encoders,
                |frac| {
                    if last_emit.elapsed() >= Duration::from_millis(250) {
                        last_emit = Instant::now();
                        let scaled = 0.25 + frac * 0.73;
                        let prog_job = ExportJobStatus {
                            id: job_prog.clone(),
                            project_id: proj_id_prog.clone(),
                            state: ExportState::Compositing,
                            progress: scaled,
                            stage_label: "Compositing timeline & -14 LUFS audio master".to_string(),
                            output_path: None,
                            error: None,
                            updated_at: Utc::now(),
                        };
                        if let Ok(mut jobs) = state_prog.inner.export_jobs.try_write() {
                            jobs.insert(job_prog.clone(), prog_job.clone());
                        }
                        state_prog.emit(EngineEvent::ExportProgress {
                            project_id: proj_id_prog.clone(),
                            job: prog_job,
                        });
                    }
                },
            )
        })
        .await;

        let _ = tokio::fs::remove_dir_all(&overlay_tmp_dir).await;

        match res {
            Ok(Ok(final_path)) => {
                let rel_key = format!(
                    "exports/{}",
                    final_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("export.mp4")
                );
                let storage = state_bg.inner.storage.clone();
                let fp_clone = final_path.clone();
                let synced_output =
                    tokio::task::spawn_blocking(move || storage.sync_file(&rel_key, &fp_clone))
                        .await
                        .ok()
                        .and_then(Result::ok)
                        .unwrap_or_else(|| final_path.to_string_lossy().to_string());
                compositing.state = ExportState::Completed;
                compositing.progress = 1.0;
                compositing.stage_label = "Export complete".to_string();
                compositing.output_path = Some(synced_output);
                compositing.updated_at = Utc::now();
            }
            Ok(Err(e)) => {
                compositing.state = ExportState::Failed;
                compositing.stage_label = "Export failed".to_string();
                compositing.error = Some(e.to_string());
                compositing.updated_at = Utc::now();
            }
            Err(e) => {
                compositing.state = ExportState::Failed;
                compositing.stage_label = "Export task panicked".to_string();
                compositing.error = Some(e.to_string());
                compositing.updated_at = Utc::now();
            }
        }

        state_bg
            .inner
            .export_jobs
            .write()
            .await
            .insert(job_id_bg, compositing.clone());
        state_bg.emit(EngineEvent::ExportProgress {
            project_id: id,
            job: compositing,
        });
    });

    Ok(initial)
}

pub async fn get_export_job(
    State(state): State<AppState>,
    AxumPath(job_id): AxumPath<String>,
) -> Result<Json<ExportJobStatus>, (StatusCode, String)> {
    let guard = state.inner.export_jobs.read().await;
    let job = guard.get(&job_id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("export job '{job_id}' not found"),
        )
    })?;
    Ok(Json(job.clone()))
}

pub async fn download_export(
    State(state): State<AppState>,
    AxumPath(job_id): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Response, (StatusCode, String)> {
    let output_str = {
        let guard = state.inner.export_jobs.read().await;
        let job = guard.get(&job_id).ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("export job '{job_id}' not found"),
            )
        })?;
        if job.state != ExportState::Completed {
            return Err((
                StatusCode::CONFLICT,
                "export job is not completed yet".to_string(),
            ));
        }
        job.output_path.clone().ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "export output path missing".to_string(),
            )
        })?
    };

    let safe_path = state
        .resolve_readable_path_async(output_str)
        .await
        .map_err(|e| (StatusCode::FORBIDDEN, e.to_string()))?;
    let file_name = safe_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("solaris_export.mp4")
        .to_string();

    stream_file_with_range(safe_path, headers, Some(&file_name)).await
}
