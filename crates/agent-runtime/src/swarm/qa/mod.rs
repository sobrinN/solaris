mod checks;
mod vision;

use std::sync::atomic::{AtomicU32, Ordering};

use chrono::Utc;
use solaris_core_ir::{AgentRole, CommandMeta, QaCheckItem, QaReport, QaSeverity, TimelineCommand};
use tokio_util::sync::CancellationToken;

use crate::backend::SolarisBackend;
use crate::openrouter::OpenRouterClient;
use crate::swarm::TrackLockGuard;

use checks::{
    check_and_fix_audio, check_and_fix_log_grade, check_and_fix_safe_zones, check_gfx_lint,
    check_timeline_pacing,
};
use vision::inspect_playback_frames;

pub async fn run_qa_pass(
    backend: &dyn SolarisBackend,
    openrouter: Option<&OpenRouterClient>,
    vision_model: Option<(&str, &[String])>,
    project_id: &str,
    auto_fix: bool,
    cancel: &CancellationToken,
    ops_counter: Option<&AtomicU32>,
) -> QaReport {
    let _ = backend
        .emit_cursor_pulse(
            project_id,
            "director-qa",
            AgentRole::Director,
            "program.stage",
            "running multimodal QA pass",
        )
        .await;

    let mut checks = Vec::new();
    let mut auto_fixes_applied: u32 = 0;

    let mut project = match backend.get_project(project_id).await {
        Ok(p) => p,
        Err(err) => {
            checks.push(QaCheckItem {
                id: "project-load".to_string(),
                category: "timeline".to_string(),
                severity: QaSeverity::Issue,
                message: format!("Failed to load project for QA: {err}"),
                time_sec: None,
                auto_fixed: false,
            });
            return finalize_report(checks, 0, 0, None);
        }
    };

    let brand = backend.get_brand(&project.brand_playbook_id).await.ok();
    let assets = backend.list_assets().await.unwrap_or_default();

    // 1. Timeline completeness, gaps & Log-to-Rec.709 recovery
    if !cancel.is_cancelled() {
        let total_clips: usize = project.tracks.iter().map(|t| t.clips.len()).sum();
        if total_clips == 0 {
            checks.push(QaCheckItem {
                id: "timeline-empty".to_string(),
                category: "timeline".to_string(),
                severity: QaSeverity::Issue,
                message: "Timeline has 0 clips placed across all 7 tracks.".to_string(),
                time_sec: Some(0.0),
                auto_fixed: false,
            });
        } else {
            check_timeline_pacing(&project, &mut checks);
            if check_and_fix_log_grade(
                backend,
                &mut project,
                brand.as_ref().and_then(|b| b.default_lut.as_deref()),
                auto_fix,
                ops_counter,
                &mut auto_fixes_applied,
                &mut checks,
            )
            .await
                && let Ok(updated) = backend.get_project(project_id).await
            {
                project = updated;
            }
        }
    }

    // 2. Audio -14 LUFS & music ducking
    if !cancel.is_cancelled()
        && check_and_fix_audio(
            backend,
            &project,
            auto_fix,
            cancel,
            ops_counter,
            &mut auto_fixes_applied,
            &mut checks,
        )
        .await
        && let Ok(updated) = backend.get_project(project_id).await
    {
        project = updated;
    }

    // 3. Social 9:16 safe-zone & caption/GFX overlap
    if !cancel.is_cancelled()
        && check_and_fix_safe_zones(
            backend,
            &mut project,
            auto_fix,
            cancel,
            ops_counter,
            &mut auto_fixes_applied,
            &mut checks,
        )
        .await
        && let Ok(updated) = backend.get_project(project_id).await
    {
        project = updated;
    }

    // 4. Deterministic GFX HTML lint
    if !cancel.is_cancelled() {
        check_gfx_lint(&project, &assets, &mut checks);
    }

    // 5. Playback frame capture & optional OpenRouter multimodal vision critique
    let (frames_inspected, multimodal_summary) = inspect_playback_frames(
        backend,
        openrouter,
        vision_model,
        &project,
        cancel,
        &mut checks,
    )
    .await;

    finalize_report(
        checks,
        frames_inspected,
        auto_fixes_applied,
        multimodal_summary,
    )
}

pub(super) async fn apply_locked_op(
    backend: &dyn SolarisBackend,
    project_id: &str,
    track_id: &str,
    command: TimelineCommand,
    reason: &str,
    ops_counter: Option<&AtomicU32>,
) -> bool {
    let Ok(guard) = TrackLockGuard::acquire_ref(
        backend,
        project_id,
        &[track_id],
        "director-qa",
        AgentRole::Director,
    )
    .await
    else {
        return false;
    };
    let ok = backend
        .apply_op(project_id, command, qa_meta(reason))
        .await
        .is_ok();
    if ok && let Some(counter) = ops_counter {
        counter.fetch_add(1, Ordering::Relaxed);
    }
    guard.release().await;
    ok
}

fn qa_meta(reason: &str) -> CommandMeta {
    CommandMeta {
        agent_id: "director-qa".to_string(),
        role: AgentRole::Director,
        reason: Some(reason.to_string()),
    }
}

fn finalize_report(
    checks: Vec<QaCheckItem>,
    frames_inspected: usize,
    auto_fixes_applied: u32,
    multimodal_summary: Option<String>,
) -> QaReport {
    let mut score: i32 = 100;
    let mut issue_count = 0usize;
    for item in &checks {
        match item.severity {
            QaSeverity::Issue => {
                issue_count += 1;
                score -= 15;
            }
            QaSeverity::Warning => {
                score -= 5;
            }
            QaSeverity::Pass | QaSeverity::Fixed => {}
        }
    }
    let clamped = score.clamp(0, 100) as u8;
    QaReport {
        passed: issue_count == 0 && clamped >= 70,
        score: clamped,
        frames_inspected,
        auto_fixes_applied,
        checks,
        multimodal_summary,
        checked_at: Utc::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{BoxFuture, CapturedFrame, KeyframeResult};
    use crate::error::AgentError;
    use serde_json::json;
    use solaris_core_ir::{
        AspectPreset, AudioSettings, BrandPlaybook, CaptionBlock, CaptionPreset, CaptionWord, Clip,
        ClipKind, ExportJobStatus, ExportRequest, GfxInstance, MediaAsset,
        MediaIntelligenceManifest, MediaKind, OpResult, Project, ProjectEngine, SavedCodeAsset,
        TranscriptWord, Transform2D,
    };
    use std::collections::HashMap;
    use tokio::sync::RwLock;

    struct QaMockBackend {
        engine: RwLock<ProjectEngine>,
    }

    impl SolarisBackend for QaMockBackend {
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
                self.engine
                    .write()
                    .await
                    .acquire_track_lock(&tid, aid, role)
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
                self.engine
                    .write()
                    .await
                    .release_track_lock(&tid, &aid)
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
            ts: f64,
        ) -> BoxFuture<'_, Result<CapturedFrame, AgentError>> {
            Box::pin(async move {
                Ok(CapturedFrame {
                    time_sec: ts,
                    width: 1080,
                    height: 1920,
                    png_base64: "iVBORw0KGgo=".to_string(),
                    note: "mock frame".to_string(),
                })
            })
        }
        fn list_brands(&self) -> BoxFuture<'_, Result<Vec<BrandPlaybook>, AgentError>> {
            Box::pin(async { Ok(vec![BrandPlaybook::solaris_default()]) })
        }
        fn get_brand(&self, _bid: &str) -> BoxFuture<'_, Result<BrandPlaybook, AgentError>> {
            Box::pin(async { Ok(BrandPlaybook::solaris_default()) })
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
    async fn qa_auto_fixes_log_grade_lufs_ducking_safezone_and_flags_bad_gfx() {
        let mut proj = Project::new_default("qa_test", AspectPreset::Vertical9x16);
        let pid = proj.id.clone();
        proj.media_pool.push(MediaAsset {
            id: "m1".to_string(),
            name: "slog3.mp4".to_string(),
            kind: MediaKind::Video,
            role_hint: "a_roll".to_string(),
            path: "/tmp/s.mp4".to_string(),
            added_at: Utc::now(),
            intelligence: MediaIntelligenceManifest {
                analyzed: true,
                duration_sec: 8.0,
                is_log_footage: true,
                has_audio: true,
                integrated_lufs: Some(-21.0),
                ..MediaIntelligenceManifest::default()
            },
        });
        proj.media_pool.push(MediaAsset {
            id: "m2".to_string(),
            name: "music.wav".to_string(),
            kind: MediaKind::Audio,
            role_hint: "music".to_string(),
            path: "/tmp/m.wav".to_string(),
            added_at: Utc::now(),
            intelligence: MediaIntelligenceManifest {
                analyzed: true,
                duration_sec: 8.0,
                has_audio: true,
                ..MediaIntelligenceManifest::default()
            },
        });

        let mut engine = ProjectEngine::new(proj);
        let meta = CommandMeta::default();
        engine
            .apply(
                TimelineCommand::PlaceClip {
                    track_id: "v1".into(),
                    clip: Clip {
                        id: "c_v1".into(),
                        label: "A-Roll".into(),
                        start_sec: 0.0,
                        duration_sec: 6.0,
                        source_in_sec: 0.0,
                        transform: Transform2D::default(),
                        audio: AudioSettings::default(),
                        payload: ClipKind::Media {
                            media_id: "m1".into(),
                            ken_burns: false,
                        },
                    },
                },
                meta.clone(),
            )
            .unwrap();
        engine
            .apply(
                TimelineCommand::PlaceClip {
                    track_id: "a2".into(),
                    clip: Clip {
                        id: "c_a2".into(),
                        label: "Beat".into(),
                        start_sec: 0.0,
                        duration_sec: 6.0,
                        source_in_sec: 0.0,
                        transform: Transform2D::default(),
                        audio: AudioSettings::default(),
                        payload: ClipKind::Media {
                            media_id: "m2".into(),
                            ken_burns: false,
                        },
                    },
                },
                meta.clone(),
            )
            .unwrap();
        engine
            .apply(
                TimelineCommand::PlaceClip {
                    track_id: "t1".into(),
                    clip: Clip {
                        id: "c_cap".into(),
                        label: "Low Cap".into(),
                        start_sec: 0.5,
                        duration_sec: 2.0,
                        source_in_sec: 0.0,
                        transform: Transform2D {
                            y: 0.45, // outside 80% safe zone -> clamped to 0.32
                            ..Transform2D::default()
                        },
                        audio: AudioSettings::default(),
                        payload: ClipKind::Caption {
                            caption: CaptionBlock {
                                words: vec![CaptionWord {
                                    text: "HI".into(),
                                    start_sec: 0.5,
                                    end_sec: 1.5,
                                }],
                                preset: CaptionPreset::NeonGlow,
                                font_family: "Sora".into(),
                                highlight_color: "#F7B733".into(),
                            },
                        },
                    },
                },
                meta.clone(),
            )
            .unwrap();
        engine
            .apply(
                TimelineCommand::PlaceGfx {
                    track_id: "g1".into(),
                    clip_id: "c_gfx".into(),
                    label: "Bad Script GFX".into(),
                    start_sec: 0.5,
                    duration_sec: 2.0,
                    gfx: GfxInstance {
                        asset_id: None,
                        props: json!({}),
                        inline_html: Some(
                            r#"<script src="https://evil.test/x.js"></script>"#.into(),
                        ),
                    },
                },
                meta,
            )
            .unwrap();

        let backend = QaMockBackend {
            engine: RwLock::new(engine),
        };
        let report = run_qa_pass(
            &backend,
            None,
            None,
            &pid,
            true,
            &CancellationToken::new(),
            None,
        )
        .await;

        assert_eq!(report.frames_inspected, 3);
        assert!(report.auto_fixes_applied >= 4);
        assert!(!report.passed);

        let guard = backend.engine.read().await;
        assert_eq!(guard.track_locks(), &HashMap::new());
        let p = guard.project();
        assert!(p.active_grade.contrast > 1.1);
        let a2_clip = &p.tracks.iter().find(|t| t.id == "a2").unwrap().clips[0];
        assert!(a2_clip.audio.ducking_enabled);
        let t1_clip = &p.tracks.iter().find(|t| t.id == "t1").unwrap().clips[0];
        assert!(t1_clip.transform.y <= 0.32);
    }

    #[tokio::test]
    async fn qa_offsets_colliding_gfx_to_opposite_band_and_uses_updated_caption_y() {
        let proj = Project::new_default("qa_overlap", AspectPreset::Vertical9x16);
        let pid = proj.id.clone();
        let mut engine = ProjectEngine::new(proj);
        let meta = CommandMeta::default();

        // Caption initially out of bounds at y = -0.45 (clamped to -0.32),
        // and GFX sitting at y = -0.28 (within 0.08 of -0.32!)
        engine
            .apply(
                TimelineCommand::PlaceClip {
                    track_id: "t1".into(),
                    clip: Clip {
                        id: "c_cap_top".into(),
                        label: "Top Cap".into(),
                        start_sec: 0.0,
                        duration_sec: 2.0,
                        source_in_sec: 0.0,
                        transform: Transform2D {
                            y: -0.45,
                            ..Transform2D::default()
                        },
                        audio: AudioSettings::default(),
                        payload: ClipKind::Caption {
                            caption: CaptionBlock {
                                words: vec![CaptionWord {
                                    text: "TOP".into(),
                                    start_sec: 0.0,
                                    end_sec: 1.5,
                                }],
                                preset: CaptionPreset::Minimal,
                                font_family: "Sora".into(),
                                highlight_color: "#F05A00".into(),
                            },
                        },
                    },
                },
                meta.clone(),
            )
            .unwrap();
        engine
            .apply(
                TimelineCommand::PlaceGfx {
                    track_id: "g1".into(),
                    clip_id: "c_gfx_top".into(),
                    label: "Top GFX".into(),
                    start_sec: 0.0,
                    duration_sec: 2.0,
                    gfx: GfxInstance {
                        asset_id: None,
                        props: json!({}),
                        inline_html: Some(
                            r#"<div class="clip" data-start="0" data-duration="2" style="color:var(--brand-ink);">Hi</div>"#
                                .into(),
                        ),
                    },
                },
                meta.clone(),
            )
            .unwrap();
        engine
            .apply(
                TimelineCommand::UpdateClipTransform {
                    track_id: "g1".into(),
                    clip_id: "c_gfx_top".into(),
                    transform: Transform2D {
                        y: -0.28,
                        ..Transform2D::default()
                    },
                },
                meta,
            )
            .unwrap();

        let backend = QaMockBackend {
            engine: RwLock::new(engine),
        };
        let report = run_qa_pass(
            &backend,
            None,
            None,
            &pid,
            true,
            &CancellationToken::new(),
            None,
        )
        .await;

        assert!(report.auto_fixes_applied >= 2);
        let guard = backend.engine.read().await;
        let p = guard.project();
        let cap_y = p.tracks.iter().find(|t| t.id == "t1").unwrap().clips[0]
            .transform
            .y;
        let gfx_y = p.tracks.iter().find(|t| t.id == "g1").unwrap().clips[0]
            .transform
            .y;
        assert!((cap_y - (-0.32)).abs() < 1e-6);
        // Because caption is in upper half (< 0.0), GFX must move to lower band (+0.22)
        assert!((gfx_y - 0.22).abs() < 1e-6);
    }

    #[tokio::test]
    async fn qa_recognizes_fx_track_grade_clip_for_log_footage() {
        let mut proj = Project::new_default("qa_fx_grade", AspectPreset::Vertical9x16);
        let pid = proj.id.clone();
        proj.media_pool.push(MediaAsset {
            id: "m1".to_string(),
            name: "slog3.mp4".to_string(),
            kind: MediaKind::Video,
            role_hint: "a_roll".to_string(),
            path: "/tmp/s.mp4".to_string(),
            added_at: Utc::now(),
            intelligence: MediaIntelligenceManifest {
                analyzed: true,
                duration_sec: 5.0,
                is_log_footage: true,
                ..MediaIntelligenceManifest::default()
            },
        });
        let mut engine = ProjectEngine::new(proj);
        let meta = CommandMeta::default();
        engine
            .apply(
                TimelineCommand::PlaceClip {
                    track_id: "v1".into(),
                    clip: Clip {
                        id: "c_v1".into(),
                        label: "Log".into(),
                        start_sec: 0.0,
                        duration_sec: 5.0,
                        source_in_sec: 0.0,
                        transform: Transform2D::default(),
                        audio: AudioSettings::default(),
                        payload: ClipKind::Media {
                            media_id: "m1".into(),
                            ken_burns: false,
                        },
                    },
                },
                meta.clone(),
            )
            .unwrap();
        engine
            .apply(
                TimelineCommand::PlaceClip {
                    track_id: "fx".into(),
                    clip: Clip {
                        id: "c_fx".into(),
                        label: "Rec709 FX".into(),
                        start_sec: 0.0,
                        duration_sec: 5.0,
                        source_in_sec: 0.0,
                        transform: Transform2D::default(),
                        audio: AudioSettings::default(),
                        payload: ClipKind::Grade {
                            grade: solaris_core_ir::ColorGrade {
                                contrast: 1.2,
                                saturation: 1.15,
                                ..solaris_core_ir::ColorGrade::default()
                            },
                        },
                    },
                },
                meta,
            )
            .unwrap();

        let backend = QaMockBackend {
            engine: RwLock::new(engine),
        };
        let report = run_qa_pass(
            &backend,
            None,
            None,
            &pid,
            true,
            &CancellationToken::new(),
            None,
        )
        .await;
        assert_eq!(report.auto_fixes_applied, 0);
        assert!(
            report
                .checks
                .iter()
                .any(|c| c.id == "log-recovery" && c.severity == QaSeverity::Pass)
        );
    }
}
