use std::path::PathBuf;
use std::process::Command;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Utc;
use flate2::read::ZlibDecoder;
use serde_json::json;
use solaris_agent_runtime::{
    AgentError, BoxFuture, CapturedFrame, KeyframeResult, SolarisBackend, lint_gfx_html,
    transcribe_audio_openrouter,
};
use solaris_core_ir::{
    AgentRole, AspectPreset, BrandPlaybook, ClipKind, CommandMeta, EngineEvent, ExportJobStatus,
    ExportRequest, MediaAsset, MediaIntelligenceManifest, MediaKind, OpResult, Project,
    ProjectEngine, SavedCodeAsset, TimelineCommand, TrackKind, TranscriptWord,
    validate_single_asset,
};
use solaris_media_engine::{
    CompositeFrameOptions, capture_composited_frame_png, capture_overlay_only_frame_png,
    extract_audio_for_stt, extract_keyframes_png, generate_llm_video_proxy,
    render_single_frame_overlays_via_chromium, transcribe_via_ffmpeg_whisper_checked,
};
use uuid::Uuid;

use crate::routes::media::spawn_media_analysis_task;
use crate::state::{AppState, save_json_atomic_async, validate_safe_id};

struct TempFileGuard(PathBuf);

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl SolarisBackend for AppState {
    fn list_projects(&self) -> BoxFuture<'_, Result<Vec<serde_json::Value>, AgentError>> {
        Box::pin(async move {
            let guard = self.inner.projects.read().await;
            let mut items: Vec<serde_json::Value> = guard
                .values()
                .map(|engine| {
                    let p = engine.project();
                    json!({
                        "id": p.id,
                        "name": p.name,
                        "revision": p.revision,
                        "aspect_preset": p.canvas.aspect_preset,
                        "duration_sec": p.duration_sec(),
                        "media_count": p.media_pool.len(),
                        "updated_at": p.updated_at,
                    })
                })
                .collect();
            items.sort_by(|a, b| b["updated_at"].as_str().cmp(&a["updated_at"].as_str()));
            Ok(items)
        })
    }

    fn create_project(
        &self,
        name: String,
        aspect_preset: Option<AspectPreset>,
        brand_playbook_id: Option<String>,
        prompt: Option<String>,
        template_id: Option<String>,
    ) -> BoxFuture<'_, Result<Project, AgentError>> {
        Box::pin(async move {
            let project = if let Some(tid) = template_id.filter(|t| !t.trim().is_empty()) {
                let assets_vec: Vec<SavedCodeAsset> =
                    self.inner.assets.read().await.values().cloned().collect();
                solaris_core_ir::instantiate_template(
                    &tid,
                    Some(name),
                    brand_playbook_id,
                    prompt,
                    aspect_preset,
                    &assets_vec,
                )
                .ok_or_else(|| AgentError::ToolError(format!("unknown template '{tid}'")))?
            } else {
                let mut project =
                    Project::new_default(name, aspect_preset.unwrap_or(AspectPreset::Vertical9x16));
                if let Some(brand_id) = brand_playbook_id {
                    project.brand_playbook_id = brand_id;
                }
                if let Some(p) = prompt {
                    project.prompt = p;
                }
                project
            };
            self.persist_project_locked(&project).await;
            let mut guard = self.inner.projects.write().await;
            guard.insert(project.id.clone(), ProjectEngine::new(project.clone()));
            Ok(project)
        })
    }

    fn get_project(&self, project_id: &str) -> BoxFuture<'_, Result<Project, AgentError>> {
        let pid = project_id.to_string();
        Box::pin(async move {
            let guard = self.inner.projects.read().await;
            let engine = guard
                .get(&pid)
                .ok_or_else(|| AgentError::ToolError(format!("project '{pid}' not found")))?;
            Ok(engine.project().clone())
        })
    }

    fn apply_op(
        &self,
        project_id: &str,
        command: TimelineCommand,
        meta: CommandMeta,
    ) -> BoxFuture<'_, Result<(OpResult, Project), AgentError>> {
        let pid = project_id.to_string();
        Box::pin(async move {
            self.validate_timeline_command(&command)
                .await
                .map_err(|(_, msg)| AgentError::ToolError(msg))?;
            let (result, project) = {
                let mut guard = self.inner.projects.write().await;
                let engine = guard
                    .get_mut(&pid)
                    .ok_or_else(|| AgentError::ToolError(format!("project '{pid}' not found")))?;
                let (result, event) = engine
                    .apply(command, meta)
                    .map_err(|e| AgentError::ToolError(e.to_string()))?;
                let project = engine.project().clone();
                self.emit(event);
                (result, project)
            };
            self.persist_project_locked(&project).await;
            Ok((result, project))
        })
    }

    fn undo(&self, project_id: &str) -> BoxFuture<'_, Result<Project, AgentError>> {
        let pid = project_id.to_string();
        Box::pin(async move {
            let project = {
                let mut guard = self.inner.projects.write().await;
                let engine = guard
                    .get_mut(&pid)
                    .ok_or_else(|| AgentError::ToolError(format!("project '{pid}' not found")))?;
                let event = engine
                    .undo()
                    .map_err(|e| AgentError::ToolError(e.to_string()))?;
                let project = engine.project().clone();
                self.emit(event);
                project
            };
            self.persist_project_locked(&project).await;
            Ok(project)
        })
    }

    fn redo(&self, project_id: &str) -> BoxFuture<'_, Result<Project, AgentError>> {
        let pid = project_id.to_string();
        Box::pin(async move {
            let project = {
                let mut guard = self.inner.projects.write().await;
                let engine = guard
                    .get_mut(&pid)
                    .ok_or_else(|| AgentError::ToolError(format!("project '{pid}' not found")))?;
                let event = engine
                    .redo()
                    .map_err(|e| AgentError::ToolError(e.to_string()))?;
                let project = engine.project().clone();
                self.emit(event);
                project
            };
            self.persist_project_locked(&project).await;
            Ok(project)
        })
    }

    fn acquire_track_lock(
        &self,
        project_id: &str,
        track_id: &str,
        agent_id: &str,
        role: AgentRole,
    ) -> BoxFuture<'_, Result<(), AgentError>> {
        let pid = project_id.to_string();
        let tid = track_id.to_string();
        let aid = agent_id.to_string();
        Box::pin(async move {
            let mut guard = self.inner.projects.write().await;
            let engine = guard
                .get_mut(&pid)
                .ok_or_else(|| AgentError::ToolError(format!("project '{pid}' not found")))?;
            let event = engine
                .acquire_track_lock(&tid, aid, role)
                .map_err(|e| AgentError::ToolError(e.to_string()))?;
            self.emit(event);
            Ok(())
        })
    }

    fn release_track_lock(
        &self,
        project_id: &str,
        track_id: &str,
        agent_id: &str,
    ) -> BoxFuture<'_, Result<(), AgentError>> {
        let pid = project_id.to_string();
        let tid = track_id.to_string();
        let aid = agent_id.to_string();
        Box::pin(async move {
            let mut guard = self.inner.projects.write().await;
            let engine = guard
                .get_mut(&pid)
                .ok_or_else(|| AgentError::ToolError(format!("project '{pid}' not found")))?;
            let event = engine
                .release_track_lock(&tid, &aid)
                .map_err(|e| AgentError::ToolError(e.to_string()))?;
            self.emit(event);
            Ok(())
        })
    }

    fn import_media_paths(
        &self,
        project_id: &str,
        paths: Vec<(String, Option<String>)>,
    ) -> BoxFuture<'_, Result<Vec<MediaAsset>, AgentError>> {
        let pid = project_id.to_string();
        Box::pin(async move {
            let mut assets = Vec::with_capacity(paths.len());
            let mut resolved_pairs = Vec::with_capacity(paths.len());
            for (raw_path, role_hint) in paths {
                let resolved = self
                    .resolve_readable_path_async(raw_path)
                    .await
                    .map_err(|e| AgentError::ToolError(e.to_string()))?;
                let name = resolved
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("media")
                    .to_string();
                let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
                let kind = match ext.as_str() {
                    "mp4" | "mov" | "webm" | "mkv" | "m4v" => MediaKind::Video,
                    "png" | "jpg" | "jpeg" | "webp" | "avif" | "gif" | "svg" => MediaKind::Image,
                    "wav" | "mp3" | "aac" | "m4a" | "ogg" | "flac" => MediaKind::Audio,
                    _ => MediaKind::Document,
                };
                let asset = MediaAsset {
                    id: Uuid::new_v4().to_string(),
                    name,
                    kind,
                    role_hint: role_hint.unwrap_or_else(|| match kind {
                        MediaKind::Video => "a_roll".to_string(),
                        MediaKind::Image => "b_roll".to_string(),
                        MediaKind::Audio => "audio".to_string(),
                        MediaKind::Document => "reference".to_string(),
                    }),
                    path: resolved.to_string_lossy().to_string(),
                    added_at: Utc::now(),
                    intelligence: MediaIntelligenceManifest::default(),
                };
                assets.push(asset.clone());
                resolved_pairs.push((asset, resolved));
            }

            let project = {
                let mut guard = self.inner.projects.write().await;
                let engine = guard
                    .get_mut(&pid)
                    .ok_or_else(|| AgentError::ToolError(format!("project '{pid}' not found")))?;
                for asset in &assets {
                    let (_, event) = engine
                        .apply(
                            TimelineCommand::AddMedia {
                                asset: asset.clone(),
                            },
                            CommandMeta::default(),
                        )
                        .map_err(|e| AgentError::ToolError(e.to_string()))?;
                    self.emit(event);
                }
                engine.project().clone()
            };
            self.persist_project_locked(&project).await;
            for (asset, resolved) in resolved_pairs {
                spawn_media_analysis_task(self.clone(), pid.clone(), asset, resolved);
            }
            Ok(assets)
        })
    }

    fn transcribe_media(
        &self,
        project_id: &str,
        media_id: &str,
        language: Option<String>,
    ) -> BoxFuture<'_, Result<Vec<TranscriptWord>, AgentError>> {
        let pid = project_id.to_string();
        let mid = media_id.to_string();
        Box::pin(async move {
            validate_safe_id(&pid).map_err(|(_, msg)| AgentError::ToolError(msg))?;
            validate_safe_id(&mid).map_err(|(_, msg)| AgentError::ToolError(msg))?;
            let asset_path = {
                let guard = self.inner.projects.read().await;
                let engine = guard
                    .get(&pid)
                    .ok_or_else(|| AgentError::ToolError(format!("project '{pid}' not found")))?;
                let asset = engine
                    .project()
                    .media_pool
                    .iter()
                    .find(|m| m.id == mid)
                    .ok_or_else(|| AgentError::ToolError(format!("media '{mid}' not found")))?;
                asset.path.clone()
            };

            let safe_path = self
                .resolve_readable_path_async(asset_path)
                .await
                .map_err(|e| AgentError::ToolError(e.to_string()))?;

            let work_dir = self.inner.workspace_dir.join("tmp_stt").join(&pid);
            let whisper_model =
                solaris_media_engine::discover_whisper_model(Some(&self.inner.workspace_dir));

            let mut words: Option<Vec<TranscriptWord>> = None;
            let mut last_stt_err: Option<String> = None;

            if let Some(ref model_path) = whisper_model {
                if self.inner.toolchain.whisper_cli_path.is_some() {
                    let tc = self.inner.toolchain.clone();
                    let sp = safe_path.clone();
                    let mp = model_path.clone();
                    let wd = work_dir.clone();
                    let lang = language.clone();
                    match tokio::task::spawn_blocking(move || {
                        solaris_media_engine::transcribe_via_whisper_cli(
                            &tc,
                            &sp,
                            &mp,
                            lang.as_deref(),
                            &wd,
                        )
                    })
                    .await
                    {
                        Ok(Ok(w)) => words = Some(w),
                        Ok(Err(e)) => {
                            let msg = format!("whisper-cli failed ({e}); falling back");
                            tracing::warn!(project_id = %pid, "{msg}");
                            last_stt_err = Some(e.to_string());
                        }
                        Err(e) => {
                            last_stt_err = Some(e.to_string());
                        }
                    }
                }

                if words.is_none() && self.inner.capabilities.has_whisper_filter {
                    let tc = self.inner.toolchain.clone();
                    let has_whisper = self.inner.capabilities.has_whisper_filter;
                    let sp = safe_path.clone();
                    let mp = model_path.clone();
                    let wd = work_dir.clone();
                    let lang = language.clone();
                    match tokio::task::spawn_blocking(move || {
                        transcribe_via_ffmpeg_whisper_checked(
                            &tc,
                            has_whisper,
                            &sp,
                            &mp,
                            lang.as_deref(),
                            &wd,
                        )
                    })
                    .await
                    {
                        Ok(Ok(w)) => words = Some(w),
                        Ok(Err(e)) => {
                            let msg = format!("ffmpeg whisper failed ({e}); falling back");
                            tracing::warn!(project_id = %pid, "{msg}");
                            last_stt_err = Some(e.to_string());
                        }
                        Err(e) => {
                            last_stt_err = Some(e.to_string());
                        }
                    }
                }
            }

            let words = if let Some(w) = words {
                w
            } else if self.inner.openrouter.is_configured() {
                let mp3_path = work_dir.join(format!("{mid}_{}.mp3", Uuid::new_v4()));
                let _mp3_guard = TempFileGuard(mp3_path.clone());
                let tc = self.inner.toolchain.clone();
                let sp = safe_path.clone();
                let mp3_clone = mp3_path.clone();
                let audio_for_stt = if self.inner.capabilities.ffmpeg_available {
                    tokio::task::spawn_blocking(move || {
                        extract_audio_for_stt(&tc, &sp, &mp3_clone)
                    })
                    .await
                    .map_err(|e| AgentError::Backend(e.to_string()))?
                    .map_err(|e| AgentError::Backend(e.to_string()))?;
                    mp3_path.clone()
                } else {
                    safe_path.clone()
                };
                transcribe_audio_openrouter(
                    &self.inner.openrouter,
                    &self.inner.model_advisor,
                    &audio_for_stt,
                    language.as_deref(),
                )
                .await?
            } else {
                let detail = last_stt_err
                    .map(|e| format!(" Last local STT error: {e}."))
                    .unwrap_or_default();
                return Err(AgentError::ToolError(format!(
                    "Transcription requires either OPENROUTER_API_KEY, or whisper-cpp (`brew install whisper-cpp`) / FFmpeg 8+ whisper filter with a GGML model (SOLARIS_WHISPER_MODEL or ~/.cache/whisper/ggml-base.bin).{detail}"
                )));
            };

            let snap = {
                let mut guard = self.inner.projects.write().await;
                if let Some(engine) = guard.get_mut(&pid)
                    && let Some(latest) = engine
                        .project()
                        .media_pool
                        .iter()
                        .find(|m| m.id == mid)
                        .map(|m| m.intelligence.clone())
                {
                    let mut updated_intel = latest;
                    updated_intel.transcript = words.clone();
                    if let Ok((_, ev)) = engine.apply(
                        TimelineCommand::UpdateMediaIntelligence {
                            media_id: mid.clone(),
                            intelligence: updated_intel.clone(),
                        },
                        CommandMeta::default(),
                    ) {
                        let project = engine.project().clone();
                        self.emit(ev);
                        self.emit(EngineEvent::MediaAnalyzed {
                            project_id: pid,
                            media_id: mid,
                            intelligence: updated_intel,
                        });
                        Some(project)
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            if let Some(project) = snap {
                self.persist_project_locked(&project).await;
            }

            Ok(words)
        })
    }

    fn extract_keyframes(
        &self,
        project_id: &str,
        media_id: &str,
        timestamps_sec: Vec<f64>,
    ) -> BoxFuture<'_, Result<Vec<KeyframeResult>, AgentError>> {
        let pid = project_id.to_string();
        let mid = media_id.to_string();
        Box::pin(async move {
            validate_safe_id(&pid).map_err(|(_, msg)| AgentError::ToolError(msg))?;
            validate_safe_id(&mid).map_err(|(_, msg)| AgentError::ToolError(msg))?;
            let asset_path = {
                let guard = self.inner.projects.read().await;
                let engine = guard
                    .get(&pid)
                    .ok_or_else(|| AgentError::ToolError(format!("project '{pid}' not found")))?;
                let asset = engine
                    .project()
                    .media_pool
                    .iter()
                    .find(|m| m.id == mid)
                    .ok_or_else(|| AgentError::ToolError(format!("media '{mid}' not found")))?;
                asset.path.clone()
            };
            let safe_path = self
                .resolve_readable_path_async(asset_path)
                .await
                .map_err(|e| AgentError::ToolError(e.to_string()))?;
            let out_dir = self
                .inner
                .workspace_dir
                .join("tmp_frames")
                .join(Uuid::new_v4().to_string());
            let out_dir_cleanup = out_dir.clone();
            let tc = self.inner.toolchain.clone();
            let pairs_res = tokio::task::spawn_blocking(move || {
                extract_keyframes_png(&tc, &safe_path, &timestamps_sec, &out_dir, 640)
            })
            .await
            .map_err(|e| AgentError::Backend(e.to_string()))?;

            let pairs = match pairs_res {
                Ok(p) => p,
                Err(e) => {
                    let _ = tokio::fs::remove_dir_all(&out_dir_cleanup).await;
                    return Err(AgentError::Backend(e.to_string()));
                }
            };

            let mut out = Vec::with_capacity(pairs.len());
            for (ts, path) in pairs {
                if let Ok(bytes) = tokio::fs::read(&path).await {
                    out.push(KeyframeResult {
                        time_sec: ts,
                        png_base64: BASE64.encode(bytes),
                    });
                }
            }
            let _ = tokio::fs::remove_dir_all(&out_dir_cleanup).await;
            Ok(out)
        })
    }

    fn capture_preview_frame(
        &self,
        project_id: &str,
        time_sec: f64,
    ) -> BoxFuture<'_, Result<CapturedFrame, AgentError>> {
        let pid = project_id.to_string();
        Box::pin(async move {
            let project = self.get_project(&pid).await?;
            let any_visual_solo = project
                .tracks
                .iter()
                .any(|t| matches!(t.kind, TrackKind::ARoll | TrackKind::BRoll) && t.solo);
            let mut target: Option<(String, f64, String, solaris_core_ir::Transform2D)> = None;
            for kind in [TrackKind::BRoll, TrackKind::ARoll] {
                if let Some(track) = project
                    .tracks
                    .iter()
                    .find(|t| t.kind == kind && !t.muted && (!any_visual_solo || t.solo))
                    && let Some(clip) = track.clips.iter().find(|c| {
                        time_sec >= c.start_sec && time_sec <= c.start_sec + c.duration_sec
                    })
                    && let ClipKind::Media { media_id, .. } = &clip.payload
                    && let Some(asset) = project.media_pool.iter().find(|m| m.id == *media_id)
                {
                    let src_t = (clip.source_in_sec + (time_sec - clip.start_sec)).max(0.0);
                    target = Some((
                        asset.path.clone(),
                        src_t,
                        asset.name.clone(),
                        clip.transform.clone(),
                    ));
                    break;
                }
            }

            let has_active_overlays = project.tracks.iter().any(|t| {
                !t.muted
                    && matches!(t.kind, TrackKind::Gfx | TrackKind::Text)
                    && t.clips
                        .iter()
                        .any(|c| time_sec >= c.start_sec && time_sec < c.start_sec + c.duration_sec)
            });

            if target.is_none() && !has_active_overlays {
                return Err(AgentError::ToolError(format!(
                    "no active video/image or GFX/caption clip at {time_sec:.2}s"
                )));
            }

            let (brand_tokens, brand_default_lut, brand_fonts) = self
                .load_brand_render_context(&project.brand_playbook_id)
                .await;

            let overlay_tmp_dir = self
                .inner
                .workspace_dir
                .join("tmp_frames")
                .join(format!("ov_{}", Uuid::new_v4()));
            let _ov_dir_guard = TempDirGuard(overlay_tmp_dir.clone());

            let mut overlay_render_error: Option<String> = None;
            let overlay_pngs =
                if has_active_overlays && self.inner.toolchain.chromium_path.is_some() {
                    let workspace_assets: Vec<SavedCodeAsset> =
                        self.inner.assets.read().await.values().cloned().collect();
                    let tc = self.inner.toolchain.clone();
                    let proj_clone = project.clone();
                    let tokens_clone = brand_tokens.clone();
                    let fonts_clone = brand_fonts.clone();
                    let ov_dir = overlay_tmp_dir.clone();
                    match tokio::task::spawn_blocking(move || {
                        render_single_frame_overlays_via_chromium(
                            &tc,
                            &proj_clone,
                            time_sec,
                            &tokens_clone,
                            &workspace_assets,
                            &fonts_clone,
                            &ov_dir,
                        )
                    })
                    .await
                    {
                        Ok(Ok(paths)) => paths,
                        Ok(Err(err)) => {
                            let msg = err.to_string();
                            tracing::warn!(
                                project_id = %pid,
                                time_sec,
                                "single-frame overlay render failed during preview capture: {msg}"
                            );
                            overlay_render_error = Some(msg);
                            Vec::new()
                        }
                        Err(err) => {
                            let msg = format!("single-frame overlay render task failed: {err}");
                            tracing::warn!(
                                project_id = %pid,
                                time_sec,
                                "{msg}"
                            );
                            overlay_render_error = Some(msg);
                            Vec::new()
                        }
                    }
                } else {
                    Vec::new()
                };

            let out_png = self
                .inner
                .workspace_dir
                .join("tmp_frames")
                .join(format!("preview_{}.png", Uuid::new_v4()));
            let _png_guard = TempFileGuard(out_png.clone());
            let canvas_w = project.canvas.width;
            let canvas_h = project.canvas.height;
            let overlay_count = overlay_pngs.len();

            let (actual_w, actual_h, note) = if let Some((raw_path, src_t, label, clip_transform)) =
                target
            {
                let safe_path = self
                    .resolve_readable_path_async(raw_path)
                    .await
                    .map_err(|e| AgentError::ToolError(e.to_string()))?;

                // Stack active FX adjustment-layer grades onto project.active_grade to match export.rs
                let mut effective_grade = project.active_grade.clone();
                if let Some(fx_clip) = project
                    .tracks
                    .iter()
                    .find(|t| t.kind == TrackKind::Grade && !t.muted)
                    .and_then(|t| {
                        t.clips.iter().find(|c| {
                            time_sec >= c.start_sec && time_sec < c.start_sec + c.duration_sec
                        })
                    })
                    && let ClipKind::Grade { grade: fx } = &fx_clip.payload
                {
                    effective_grade.lift.r += fx.lift.r;
                    effective_grade.lift.g += fx.lift.g;
                    effective_grade.lift.b += fx.lift.b;
                    effective_grade.gamma.r += fx.gamma.r;
                    effective_grade.gamma.g += fx.gamma.g;
                    effective_grade.gamma.b += fx.gamma.b;
                    effective_grade.gain.r += fx.gain.r;
                    effective_grade.gain.g += fx.gain.g;
                    effective_grade.gain.b += fx.gain.b;
                    effective_grade.temp += fx.temp;
                    effective_grade.tint += fx.tint;
                    effective_grade.contrast *= fx.contrast;
                    effective_grade.saturation *= fx.saturation;
                    effective_grade.exposure += fx.exposure;
                    effective_grade.brightness += fx.brightness;
                    if fx.lut_path.as_deref().is_some_and(|s| !s.trim().is_empty()) {
                        effective_grade.lut_path = fx.lut_path.clone();
                        effective_grade.lut_intensity = fx.lut_intensity;
                    }
                }

                if effective_grade.lut_path.is_none() {
                    effective_grade.lut_path = brand_default_lut;
                }

                let resolved_lut = match effective_grade.lut_path.as_deref() {
                    Some(raw_lut) if !raw_lut.trim().is_empty() => self
                        .resolve_readable_path_async(raw_lut.to_string())
                        .await
                        .ok(),
                    _ => None,
                };

                let tc = self.inner.toolchain.clone();
                let png_clone = out_png.clone();
                let (_, w, h) = tokio::task::spawn_blocking(move || {
                    capture_composited_frame_png(
                        &tc,
                        &safe_path,
                        src_t,
                        &png_clone,
                        &CompositeFrameOptions {
                            canvas_width: canvas_w,
                            canvas_height: canvas_h,
                            max_width: Some(720),
                            transform: Some(&clip_transform),
                            grade: Some(&effective_grade),
                            resolved_lut: resolved_lut.as_deref(),
                            overlay_pngs: &overlay_pngs,
                        },
                    )
                })
                .await
                .map_err(|e| AgentError::Backend(e.to_string()))?
                .map_err(|e| AgentError::Backend(e.to_string()))?;

                let n = if overlay_count > 0 {
                    format!(
                        "Composited frame ({w}x{h}, {overlay_count} overlays) from '{label}' at source {src_t:.2}s"
                    )
                } else {
                    format!(
                        "Composited graded frame ({w}x{h}) from '{label}' at source {src_t:.2}s"
                    )
                };
                (w, h, n)
            } else {
                if overlay_pngs.is_empty() {
                    let detail = if let Some(err_msg) = overlay_render_error {
                        format!(
                            "programmatic overlays active at {time_sec:.2}s, but overlay rendering failed: {err_msg}"
                        )
                    } else if self.inner.toolchain.chromium_path.is_none() {
                        format!(
                            "programmatic overlays active at {time_sec:.2}s, but headless Chromium is unavailable to render preview frames"
                        )
                    } else {
                        format!(
                            "programmatic overlays active at {time_sec:.2}s produced no renderable overlay frames"
                        )
                    };
                    return Err(AgentError::ToolError(detail));
                }
                let tc = self.inner.toolchain.clone();
                let png_clone = out_png.clone();
                let bg_hex = brand_tokens.bg.clone();
                let (_, w, h) = tokio::task::spawn_blocking(move || {
                    capture_overlay_only_frame_png(
                        &tc,
                        &png_clone,
                        canvas_w,
                        canvas_h,
                        Some(720),
                        &bg_hex,
                        &overlay_pngs,
                    )
                })
                .await
                .map_err(|e| AgentError::Backend(e.to_string()))?
                .map_err(|e| AgentError::Backend(e.to_string()))?;
                (
                    w,
                    h,
                    format!(
                        "Programmatic GFX/caption frame ({w}x{h}, {overlay_count} layer(s)) at {time_sec:.2}s"
                    ),
                )
            };

            let bytes = tokio::fs::read(&out_png)
                .await
                .map_err(|e| AgentError::Backend(e.to_string()))?;
            let _ = tokio::fs::remove_file(&out_png).await;
            let _ = tokio::fs::remove_dir_all(&overlay_tmp_dir).await;

            Ok(CapturedFrame {
                time_sec,
                width: actual_w,
                height: actual_h,
                png_base64: BASE64.encode(bytes),
                note,
            })
        })
    }

    fn list_brands(&self) -> BoxFuture<'_, Result<Vec<BrandPlaybook>, AgentError>> {
        Box::pin(async move {
            let guard = self.inner.brands.read().await;
            let mut items: Vec<BrandPlaybook> = guard.values().cloned().collect();
            items.sort_by(|a, b| a.id.cmp(&b.id));
            Ok(items)
        })
    }

    fn get_brand(&self, brand_id: &str) -> BoxFuture<'_, Result<BrandPlaybook, AgentError>> {
        let bid = brand_id.to_string();
        Box::pin(async move {
            let guard = self.inner.brands.read().await;
            guard
                .get(&bid)
                .cloned()
                .ok_or_else(|| AgentError::ToolError(format!("brand '{bid}' not found")))
        })
    }

    fn list_assets(&self) -> BoxFuture<'_, Result<Vec<SavedCodeAsset>, AgentError>> {
        Box::pin(async move {
            let guard = self.inner.assets.read().await;
            let mut items: Vec<SavedCodeAsset> = guard.values().cloned().collect();
            items.sort_by(|a, b| a.id.cmp(&b.id));
            Ok(items)
        })
    }

    fn save_asset(
        &self,
        asset: SavedCodeAsset,
    ) -> BoxFuture<'_, Result<SavedCodeAsset, AgentError>> {
        Box::pin(async move {
            validate_safe_id(&asset.id).map_err(|(_, msg)| AgentError::ToolError(msg))?;
            validate_single_asset(&asset).map_err(AgentError::ToolError)?;
            let lint = lint_gfx_html(&asset.html_source);
            if !lint.valid {
                return Err(AgentError::ToolError(format!(
                    "GFX HTML failed lint: {}",
                    lint.errors.join("; ")
                )));
            }
            let path = self
                .inner
                .workspace_dir
                .join("assets")
                .join(format!("{}.json", asset.id));
            let _persist = self.inner.asset_persist_lock.lock().await;
            save_json_atomic_async(asset.clone(), path)
                .await
                .map_err(AgentError::Backend)?;
            self.undismiss_starter_locked(&asset.id)
                .await
                .map_err(AgentError::Backend)?;
            self.inner
                .assets
                .write()
                .await
                .insert(asset.id.clone(), asset.clone());
            drop(_persist);
            self.emit_assets_changed("saved", Some(&asset.id));
            Ok(asset)
        })
    }

    fn start_export(
        &self,
        project_id: &str,
        req: ExportRequest,
    ) -> BoxFuture<'_, Result<ExportJobStatus, AgentError>> {
        let pid = project_id.to_string();
        Box::pin(async move {
            crate::routes::exports::enqueue_export_job(self, &pid, req)
                .await
                .map_err(|(_, msg)| AgentError::ToolError(msg))
        })
    }

    fn emit_cursor_pulse(
        &self,
        project_id: &str,
        agent_id: &str,
        role: AgentRole,
        ui_anchor: &str,
        label: &str,
    ) -> BoxFuture<'_, Result<(), AgentError>> {
        self.emit(EngineEvent::CursorPulse {
            project_id: project_id.to_string(),
            agent_id: agent_id.to_string(),
            role,
            ui_anchor: ui_anchor.to_string(),
            label: label.to_string(),
        });
        Box::pin(async { Ok(()) })
    }

    fn emit_playhead_seek(
        &self,
        project_id: &str,
        time_sec: f64,
        agent_id: &str,
        role: AgentRole,
    ) -> BoxFuture<'_, Result<(), AgentError>> {
        self.emit(EngineEvent::PlayheadSeek {
            project_id: project_id.to_string(),
            time_sec,
            agent_id: agent_id.to_string(),
            role,
        });
        Box::pin(async { Ok(()) })
    }

    fn read_document_excerpt(
        &self,
        project_id: &str,
        media_id: &str,
        max_chars: usize,
    ) -> BoxFuture<'_, Result<Option<String>, AgentError>> {
        let pid = project_id.to_string();
        let mid = media_id.to_string();
        Box::pin(async move {
            let asset_path = {
                let guard = self.inner.projects.read().await;
                let Some(engine) = guard.get(&pid) else {
                    return Ok(None);
                };
                let Some(asset) = engine
                    .project()
                    .media_pool
                    .iter()
                    .find(|m| m.id == mid && m.kind == MediaKind::Document)
                else {
                    return Ok(None);
                };
                asset.path.clone()
            };
            if asset_path.trim().is_empty() {
                return Ok(None);
            }
            let safe_path = self
                .resolve_readable_path_async(asset_path)
                .await
                .map_err(|e| AgentError::ToolError(e.to_string()))?;
            let capped_chars = max_chars.clamp(64, 16_000);
            let excerpt = tokio::task::spawn_blocking(move || {
                extract_document_text(&safe_path, capped_chars)
            })
            .await
            .map_err(|e| AgentError::Backend(e.to_string()))?;
            Ok(excerpt)
        })
    }

    fn prepare_llm_video_proxy(
        &self,
        project_id: &str,
        media_id: &str,
        max_bytes: usize,
    ) -> BoxFuture<'_, Result<Option<String>, AgentError>> {
        let pid = project_id.to_string();
        let mid = media_id.to_string();
        Box::pin(async move {
            if !self.inner.capabilities.ffmpeg_available {
                return Ok(None);
            }
            let asset_path = {
                let guard = self.inner.projects.read().await;
                let Some(engine) = guard.get(&pid) else {
                    return Ok(None);
                };
                let Some(asset) = engine
                    .project()
                    .media_pool
                    .iter()
                    .find(|m| m.id == mid && m.kind == MediaKind::Video)
                else {
                    return Ok(None);
                };
                asset.path.clone()
            };
            if asset_path.trim().is_empty() {
                return Ok(None);
            }
            let safe_path = match self.resolve_readable_path_async(asset_path).await {
                Ok(p) => p,
                Err(_) => return Ok(None),
            };
            let out_mp4 = self
                .inner
                .workspace_dir
                .join("tmp_frames")
                .join(format!("llm_proxy_{}.mp4", Uuid::new_v4()));
            let _mp4_guard = TempFileGuard(out_mp4.clone());
            let out_clone = out_mp4.clone();
            let tc = self.inner.toolchain.clone();
            let res = tokio::task::spawn_blocking(move || {
                generate_llm_video_proxy(&tc, &safe_path, &out_clone, 45.0)
            })
            .await;

            let encoded = match res {
                Ok(Ok(_)) => match tokio::fs::read(&out_mp4).await {
                    Ok(bytes) if !bytes.is_empty() && bytes.len() <= max_bytes => {
                        Some(BASE64.encode(bytes))
                    }
                    _ => None,
                },
                _ => None,
            };
            Ok(encoded)
        })
    }

    fn read_media_image_base64(
        &self,
        project_id: &str,
        media_id: &str,
    ) -> BoxFuture<'_, Result<Option<(String, String)>, AgentError>> {
        let pid = project_id.to_string();
        let mid = media_id.to_string();
        Box::pin(async move {
            let asset_path = {
                let guard = self.inner.projects.read().await;
                let Some(engine) = guard.get(&pid) else {
                    return Ok(None);
                };
                let Some(asset) = engine
                    .project()
                    .media_pool
                    .iter()
                    .find(|m| m.id == mid && m.kind == MediaKind::Image)
                else {
                    return Ok(None);
                };
                asset.path.clone()
            };
            if asset_path.trim().is_empty() {
                return Ok(None);
            }
            let safe_path = match self.resolve_readable_path_async(asset_path).await {
                Ok(p) => p,
                Err(_) => return Ok(None),
            };
            let ext = safe_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let direct_mime = match ext.as_str() {
                "png" => Some("image/png"),
                "jpg" | "jpeg" => Some("image/jpeg"),
                "webp" => Some("image/webp"),
                _ => None,
            };
            if let Some(mime) = direct_mime
                && let Ok(meta) = tokio::fs::metadata(&safe_path).await
                && meta.is_file()
                && meta.len() > 0
                && meta.len() <= 512 * 1024
                && let Ok(bytes) = tokio::fs::read(&safe_path).await
            {
                return Ok(Some((mime.to_string(), BASE64.encode(bytes))));
            }

            if let Some(ffmpeg) = self.inner.toolchain.ffmpeg_path.clone() {
                let out_jpg = self
                    .inner
                    .workspace_dir
                    .join("tmp_frames")
                    .join(format!("img_{}.jpg", Uuid::new_v4()));
                let _jpg_guard = TempFileGuard(out_jpg.clone());
                let out_clone = out_jpg.clone();
                let sp_clone = safe_path.clone();
                let ok = tokio::task::spawn_blocking(move || {
                    if let Some(parent) = out_clone.parent()
                        && !parent.as_os_str().is_empty()
                    {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    Command::new(&ffmpeg)
                        .args(["-y", "-hide_banner", "-v", "error", "-i"])
                        .arg(&sp_clone)
                        .args([
                            "-vf",
                            "scale='min(768,iw)':-2",
                            "-frames:v",
                            "1",
                            "-q:v",
                            "4",
                        ])
                        .arg(&out_clone)
                        .output()
                        .is_ok_and(|o| o.status.success())
                })
                .await
                .unwrap_or(false);
                if ok
                    && let Ok(bytes) = tokio::fs::read(&out_jpg).await
                    && !bytes.is_empty()
                    && bytes.len() <= 2 * 1024 * 1024
                {
                    return Ok(Some(("image/jpeg".to_string(), BASE64.encode(bytes))));
                }
            }

            if let Some(mime) = direct_mime
                && let Ok(meta) = tokio::fs::metadata(&safe_path).await
                && meta.is_file()
                && meta.len() > 0
                && meta.len() <= 2 * 1024 * 1024
                && let Ok(bytes) = tokio::fs::read(&safe_path).await
            {
                return Ok(Some((mime.to_string(), BASE64.encode(bytes))));
            }
            Ok(None)
        })
    }

    fn read_media_audio_base64(
        &self,
        project_id: &str,
        media_id: &str,
        max_sec: f64,
    ) -> BoxFuture<'_, Result<Option<(String, String)>, AgentError>> {
        let pid = project_id.to_string();
        let mid = media_id.to_string();
        Box::pin(async move {
            let Some(ffmpeg) = self.inner.toolchain.ffmpeg_path.clone() else {
                return Ok(None);
            };
            let asset_path =
                {
                    let guard = self.inner.projects.read().await;
                    let Some(engine) = guard.get(&pid) else {
                        return Ok(None);
                    };
                    let Some(asset) = engine.project().media_pool.iter().find(|m| {
                        m.id == mid && matches!(m.kind, MediaKind::Audio | MediaKind::Video)
                    }) else {
                        return Ok(None);
                    };
                    asset.path.clone()
                };
            if asset_path.trim().is_empty() {
                return Ok(None);
            }
            let safe_path = match self.resolve_readable_path_async(asset_path).await {
                Ok(p) => p,
                Err(_) => return Ok(None),
            };
            let dur = if max_sec.is_finite() && max_sec > 0.0 {
                max_sec.clamp(1.0, 60.0)
            } else {
                25.0
            };
            let out_mp3 = self
                .inner
                .workspace_dir
                .join("tmp_stt")
                .join(format!("audio_{}.mp3", Uuid::new_v4()));
            let _mp3_guard = TempFileGuard(out_mp3.clone());
            let out_clone = out_mp3.clone();
            let ok = tokio::task::spawn_blocking(move || {
                if let Some(parent) = out_clone.parent()
                    && !parent.as_os_str().is_empty()
                {
                    let _ = std::fs::create_dir_all(parent);
                }
                Command::new(&ffmpeg)
                    .args(["-y", "-hide_banner", "-v", "error", "-i"])
                    .arg(&safe_path)
                    .args([
                        "-t",
                        &format!("{dur:.3}"),
                        "-vn",
                        "-ac",
                        "1",
                        "-ar",
                        "22050",
                        "-b:a",
                        "64k",
                        "-f",
                        "mp3",
                    ])
                    .arg(&out_clone)
                    .output()
                    .is_ok_and(|o| o.status.success())
            })
            .await
            .unwrap_or(false);
            if !ok {
                return Ok(None);
            }
            match tokio::fs::read(&out_mp3).await {
                Ok(bytes) if !bytes.is_empty() && bytes.len() <= 2 * 1024 * 1024 => {
                    Ok(Some(("mp3".to_string(), BASE64.encode(bytes))))
                }
                _ => Ok(None),
            }
        })
    }

    fn spawn_release_track_lock(&self, project_id: &str, track_id: &str, agent_id: &str) {
        let state = self.clone();
        let pid = project_id.to_string();
        let tid = track_id.to_string();
        let aid = agent_id.to_string();
        tokio::spawn(async move {
            let _ = state.release_track_lock(&pid, &tid, &aid).await;
        });
    }

    fn emit_log(&self, entry: solaris_core_ir::LogEntry) {
        self.push_log(entry);
    }
}

fn extract_document_text(path: &std::path::Path, max_chars: usize) -> Option<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    let _ = file.by_ref().take(512 * 1024).read_to_end(&mut buf).ok()?;
    if buf.is_empty() {
        return None;
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let raw_text = if ext == "pdf" || buf.starts_with(b"%PDF") {
        extract_pdf_text(&buf)
    } else {
        String::from_utf8_lossy(&buf).into_owned()
    };

    let cleaned = raw_text.trim();
    if cleaned.is_empty() {
        return None;
    }
    Some(cleaned.chars().take(max_chars).collect())
}

/// Extracts text from both uncompressed PDF text blocks and `/FlateDecode` zlib-compressed streams.
pub(crate) fn extract_pdf_text(bytes: &[u8]) -> String {
    use std::io::Read;

    const MAX_STREAM_INFLATE_BYTES: u64 = 256 * 1024;
    const MAX_TOTAL_INFLATE_BYTES: usize = 2 * 1024 * 1024;

    let mut combined = extract_pdf_literal_text(bytes);
    let mut total_inflated = 0usize;
    let mut pos = 0usize;
    let len = bytes.len();

    while pos + 6 < len && combined.len() < 32_768 && total_inflated < MAX_TOTAL_INFLATE_BYTES {
        let Some(rel) = bytes[pos..].windows(6).position(|w| w == b"stream") else {
            break;
        };
        let kw_start = pos + rel;
        let prev_is_end = kw_start >= 3 && &bytes[kw_start - 3..kw_start] == b"end";
        let after_kw = kw_start + 6;
        pos = after_kw;
        if prev_is_end || after_kw >= len {
            continue;
        }
        let data_start = if bytes[after_kw] == b'\n' {
            after_kw + 1
        } else if after_kw + 1 < len && bytes[after_kw] == b'\r' && bytes[after_kw + 1] == b'\n' {
            after_kw + 2
        } else {
            continue;
        };

        let Some(end_rel) = bytes[data_start..]
            .windows(9)
            .position(|w| w == b"endstream")
        else {
            break;
        };
        let mut data_end = data_start + end_rel;
        pos = data_end + 9;
        while data_end > data_start
            && (bytes[data_end - 1] == b'\n' || bytes[data_end - 1] == b'\r')
        {
            data_end -= 1;
        }
        let raw_stream = &bytes[data_start..data_end];
        if raw_stream.len() < 2 {
            continue;
        }

        let mut decoder = ZlibDecoder::new(raw_stream).take(MAX_STREAM_INFLATE_BYTES);
        let mut inflated = Vec::new();
        if decoder.read_to_end(&mut inflated).is_ok() && !inflated.is_empty() {
            total_inflated = total_inflated.saturating_add(inflated.len());
            let chunk = extract_pdf_literal_text(&inflated);
            let trimmed = chunk.trim();
            if !trimmed.is_empty() {
                if !combined.is_empty() && !combined.ends_with('\n') {
                    combined.push('\n');
                }
                combined.push_str(trimmed);
            }
        }
    }

    combined
}

/// Lightweight zero-dependency PDF text extractor reading parenthesized PDF string literals
/// inside text blocks (`BT ... ET`) and fallback printable text streams.
fn extract_pdf_literal_text(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0usize;
    let len = bytes.len();
    let mut in_text_obj = false;

    while i < len && out.len() < 32_768 {
        if i + 2 <= len && &bytes[i..i + 2] == b"BT" {
            let prev_ok = i == 0 || bytes[i - 1].is_ascii_whitespace();
            let next_ok = i + 2 == len || bytes[i + 2].is_ascii_whitespace();
            if prev_ok && next_ok {
                in_text_obj = true;
                i += 2;
                continue;
            }
        }
        if i + 2 <= len && &bytes[i..i + 2] == b"ET" {
            let prev_ok = i == 0 || bytes[i - 1].is_ascii_whitespace();
            let next_ok = i + 2 == len || bytes[i + 2].is_ascii_whitespace();
            if prev_ok && next_ok {
                in_text_obj = false;
                if !out.ends_with('\n') && !out.is_empty() {
                    out.push('\n');
                }
                i += 2;
                continue;
            }
        }
        if in_text_obj && bytes[i] == b'(' {
            i += 1;
            let mut depth = 1usize;
            let mut escaped = false;
            let mut token = String::new();
            while i < len && depth > 0 {
                let b = bytes[i];
                i += 1;
                if escaped {
                    match b {
                        b'n' => token.push('\n'),
                        b'r' => token.push(' '),
                        b't' => token.push(' '),
                        b'(' | b')' | b'\\' => token.push(b as char),
                        _ => {}
                    }
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == b'(' {
                    depth += 1;
                    token.push('(');
                } else if b == b')' {
                    depth -= 1;
                    if depth > 0 {
                        token.push(')');
                    }
                } else if (0x20..=0x7E).contains(&b) {
                    token.push(b as char);
                }
            }
            if !token.trim().is_empty() {
                if !out.is_empty() && !out.ends_with([' ', '\n']) {
                    out.push(' ');
                }
                out.push_str(token.trim());
            }
            continue;
        }
        i += 1;
    }

    out
}
