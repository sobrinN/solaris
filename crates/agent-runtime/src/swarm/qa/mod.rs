mod checks;
mod repair;
mod vision;

use std::collections::HashSet;
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
use repair::repair_gfx_clips_from_vision;
use vision::inspect_playback_frames;

#[derive(Debug, Clone, Default)]
pub struct QaPassOptions<'a> {
    pub specialist_errors: &'a [String],
    pub director_supports_reasoning: bool,
}

pub async fn run_qa_pass(
    backend: &dyn SolarisBackend,
    openrouter: Option<&OpenRouterClient>,
    vision_model: Option<(&str, &[String])>,
    project_id: &str,
    auto_fix: bool,
    cancel: &CancellationToken,
    ops_counter: Option<&AtomicU32>,
) -> QaReport {
    run_qa_pass_with_options(
        backend,
        openrouter,
        vision_model,
        project_id,
        auto_fix,
        cancel,
        ops_counter,
        QaPassOptions::default(),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn run_qa_pass_with_options(
    backend: &dyn SolarisBackend,
    openrouter: Option<&OpenRouterClient>,
    vision_model: Option<(&str, &[String])>,
    project_id: &str,
    auto_fix: bool,
    cancel: &CancellationToken,
    ops_counter: Option<&AtomicU32>,
    options: QaPassOptions<'_>,
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

    for (idx, err) in options.specialist_errors.iter().enumerate() {
        if !err.trim().is_empty() {
            checks.push(QaCheckItem {
                id: format!("specialist-error-{idx}"),
                category: "swarm".to_string(),
                severity: QaSeverity::Issue,
                message: format!("Specialist reported an error: {err}"),
                time_sec: None,
                auto_fixed: false,
            });
        }
    }

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
            &assets,
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
    let mut vision_res = inspect_playback_frames(
        backend,
        openrouter,
        vision_model,
        options.director_supports_reasoning,
        &project,
        None,
        cancel,
        &mut checks,
    )
    .await;
    let mut total_frames_inspected = vision_res.frames_inspected;

    // 6. Closed-loop visual auto-repair: when Enhance/auto_fix is enabled and Vision QA
    // identified fixable G1 clips, patch their HTML in place and re-verify ONLY the repaired
    // clips (up to 2 rounds: round 1 fixes all reported defects; round 2 retries ONLY remaining
    // critical "issue" clips with an escalated structural-fix prompt).
    const MAX_VISUAL_REPAIR_ROUNDS: usize = 2;
    let mut repaired_clip_ids: HashSet<String> = HashSet::new();
    if auto_fix
        && !cancel.is_cancelled()
        && let (Some(client), Some(vmodel)) = (openrouter, vision_model)
        && client.is_configured()
    {
        for repair_round in 1..=MAX_VISUAL_REPAIR_ROUNDS {
            if cancel.is_cancelled() {
                break;
            }
            let active_fixes = if repair_round == 1 {
                vision_res.clip_fixes.clone()
            } else {
                repair::filter_remaining_issue_fixes(&vision_res.clip_fixes)
            };
            if active_fixes.is_empty() {
                break;
            }

            let round_repaired_ids = repair_gfx_clips_from_vision(
                backend,
                client,
                vmodel,
                options.director_supports_reasoning,
                &project,
                brand.as_ref(),
                &active_fixes,
                &vision_res.captured_by_clip,
                repair_round,
                cancel,
                ops_counter,
                &mut repaired_clip_ids,
                &mut auto_fixes_applied,
                &mut checks,
            )
            .await;

            if round_repaired_ids.is_empty() || cancel.is_cancelled() {
                break;
            }
            let Ok(updated) = backend.get_project(project_id).await else {
                break;
            };
            project = updated;

            let round_set: HashSet<String> = round_repaired_ids.into_iter().collect();
            let mut verify_checks = Vec::new();
            let verified = inspect_playback_frames(
                backend,
                openrouter,
                vision_model,
                options.director_supports_reasoning,
                &project,
                Some(&round_set),
                cancel,
                &mut verify_checks,
            )
            .await;

            total_frames_inspected = total_frames_inspected.max(verified.frames_inspected);
            for (cid, b64) in verified.captured_by_clip {
                vision_res.captured_by_clip.insert(cid, b64);
            }

            let mut untouched_fixes = Vec::new();
            let mut seen_untouched: HashSet<(String, String)> = HashSet::new();
            for fix in vision_res.clip_fixes {
                let cid = fix.clip_id.trim().to_string();
                if !round_set.contains(&cid)
                    && seen_untouched.insert((cid, fix.defect.trim().to_string()))
                {
                    untouched_fixes.push(fix);
                }
            }

            checks.retain(|c| {
                !c.id.starts_with("vision-issue-")
                    && !c.id.starts_with("vision-warn-")
                    && !c.id.starts_with("vision-unresolved-")
            });
            for (idx, f) in untouched_fixes.iter().enumerate() {
                checks.push(QaCheckItem {
                    id: format!("vision-unresolved-{idx}"),
                    category: "visual".to_string(),
                    severity: if f.severity.eq_ignore_ascii_case("issue") {
                        QaSeverity::Issue
                    } else {
                        QaSeverity::Warning
                    },
                    message: format!("Clip '{}': {}", f.clip_id, f.defect),
                    time_sec: None,
                    auto_fixed: false,
                });
            }
            checks.extend(verify_checks);

            untouched_fixes.extend(verified.clip_fixes);
            vision_res.clip_fixes = untouched_fixes;
            if verified.multimodal_summary.is_some() {
                vision_res.multimodal_summary = verified.multimodal_summary;
            }
        }
    }

    if cancel.is_cancelled() {
        checks.push(QaCheckItem {
            id: "qa-cancelled".to_string(),
            category: "qa".to_string(),
            severity: QaSeverity::Warning,
            message: "QA pass was cancelled before completion.".to_string(),
            time_sec: None,
            auto_fixed: false,
        });
    }

    finalize_report(
        checks,
        total_frames_inspected,
        auto_fixes_applied,
        vision_res.multimodal_summary,
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
    let cancelled = checks.iter().any(|c| c.id == "qa-cancelled");
    for item in &checks {
        match item.severity {
            QaSeverity::Issue => {
                issue_count += 1;
                score -= if item.id == "visuals-empty" || item.id == "timeline-empty" {
                    40
                } else {
                    15
                };
            }
            QaSeverity::Warning => {
                score -= 5;
            }
            QaSeverity::Pass | QaSeverity::Fixed => {}
        }
    }
    if issue_count > 0 {
        score = score.min(65);
    }
    let clamped = score.clamp(0, 100) as u8;
    QaReport {
        passed: !cancelled && issue_count == 0 && clamped >= 70,
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

    #[tokio::test]
    async fn qa_fails_empty_visuals_preserves_full_canvas_gfx_and_parses_vision_issues() {
        // 1. Captions-only timeline with 0 visuals on V1/V2/G1 must fail QA with low score
        let project = Project::new_default("blank_canvas", AspectPreset::Vertical9x16);
        let pid = project.id.clone();
        let mut engine = ProjectEngine::new(project);
        let meta = CommandMeta {
            agent_id: "seed".into(),
            role: AgentRole::User,
            reason: Some("seed".into()),
        };
        engine
            .apply(
                TimelineCommand::PlaceClip {
                    track_id: "t1".into(),
                    clip: Clip {
                        id: "c_cap_1".into(),
                        label: "Skills make agents".into(),
                        start_sec: 0.0,
                        duration_sec: 4.0,
                        source_in_sec: 0.0,
                        transform: Transform2D {
                            y: 0.24,
                            ..Transform2D::default()
                        },
                        audio: AudioSettings::default(),
                        payload: ClipKind::Caption {
                            caption: CaptionBlock {
                                words: vec![],
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

        let backend = QaMockBackend {
            engine: RwLock::new(engine),
        };
        let errs = vec!["gfx-01: truncated JSON".to_string()];
        let report = run_qa_pass_with_options(
            &backend,
            None,
            None,
            &pid,
            true,
            &CancellationToken::new(),
            None,
            QaPassOptions {
                specialist_errors: &errs,
                director_supports_reasoning: false,
            },
        )
        .await;
        assert!(
            !report.passed,
            "blank canvas + specialist error must fail QA"
        );
        assert!(
            report.score <= 45,
            "expected low QA score on blank canvas, got {}",
            report.score
        );
        assert!(
            report
                .checks
                .iter()
                .any(|c| c.id == "visuals-empty" && c.severity == QaSeverity::Issue)
        );
        assert!(
            report
                .checks
                .iter()
                .any(|c| c.id == "specialist-error-0" && c.severity == QaSeverity::Issue)
        );

        // 2. Full-canvas G1 scene (100vw/100vh) overlapping caption at y=0.0 must NOT be shifted
        backend
            .apply_op(
                &pid,
                TimelineCommand::PlaceGfx {
                    track_id: "g1".into(),
                    clip_id: "g_full".into(),
                    label: "Full Canvas".into(),
                    start_sec: 0.0,
                    duration_sec: 4.0,
                    gfx: GfxInstance {
                        asset_id: None,
                        props: json!({}),
                        inline_html: Some(
                            "<div class=\"clip\" data-start=\"0\" data-duration=\"4\" style=\"position:fixed;inset:0;width:100vw;height:100vh;\">Scene</div>"
                                .into(),
                        ),
                    },
                },
                meta.clone(),
            )
            .await
            .unwrap();
        backend
            .apply_op(
                &pid,
                TimelineCommand::UpdateClipTransform {
                    track_id: "t1".into(),
                    clip_id: "c_cap_1".into(),
                    transform: Transform2D {
                        y: 0.0,
                        ..Transform2D::default()
                    },
                },
                meta,
            )
            .await
            .unwrap();
        let report_full = run_qa_pass(
            &backend,
            None,
            None,
            &pid,
            true,
            &CancellationToken::new(),
            None,
        )
        .await;
        assert!(report_full.passed);
        let updated = backend.get_project(&pid).await.unwrap();
        let g_clip = updated
            .tracks
            .iter()
            .find(|t| t.id == "g1")
            .unwrap()
            .clips
            .iter()
            .find(|c| c.id == "g_full")
            .unwrap();
        assert!(
            (g_clip.transform.y - 0.0).abs() < 1e-6,
            "full-canvas G1 scene must stay at y=0.0, got {}",
            g_clip.transform.y
        );

        // 3. Vision QA parser maps critical issues to QaSeverity::Issue and warnings to Warning
        let (summary, vchecks) = vision::parse_vision_qa_output(
            r#"{"summary":"Black screen","issues":["Pitch-black background with no scene graphics"],"warnings":["Caption slightly small"]}"#,
        );
        assert_eq!(summary.as_deref(), Some("Black screen"));
        assert_eq!(vchecks.len(), 2);
        assert_eq!(vchecks[0].severity, QaSeverity::Issue);
        assert_eq!(vchecks[1].severity, QaSeverity::Warning);
    }

    #[test]
    fn pacing_early_gap_does_not_corrupt_g1_coverage_accumulation() {
        let mut proj = Project::new_default("gap_coverage", AspectPreset::Vertical9x16);
        let g1 = proj.tracks.iter_mut().find(|t| t.id == "g1").unwrap();
        // Clip 1: 0.0..2.0, 0.5s gap (2.0..2.5), Clip 2: 2.5..20.0 -> 19.5s / 20.0s = 97.5% coverage
        for (id, start, dur) in [("g1-a", 0.0, 2.0), ("g1-b", 2.5, 17.5)] {
            g1.clips.push(Clip {
                id: id.into(),
                label: id.into(),
                start_sec: start,
                duration_sec: dur,
                source_in_sec: 0.0,
                transform: Transform2D::default(),
                audio: AudioSettings::default(),
                payload: ClipKind::Gfx {
                    gfx: GfxInstance {
                        asset_id: None,
                        props: json!({}),
                        inline_html: Some(
                            r#"<div class="clip" data-start="0" data-duration="2">Scene</div>"#
                                .into(),
                        ),
                    },
                },
            });
        }
        let mut checks = Vec::new();
        checks::check_timeline_pacing(&proj, &mut checks);
        assert!(
            checks
                .iter()
                .any(|c| c.id.starts_with("g1-gap-") && c.severity == QaSeverity::Warning),
            "expected g1 gap warning"
        );
        assert!(
            !checks.iter().any(|c| c.id == "g1-coverage-low"),
            "97.5% coverage must NOT emit g1-coverage-low, got: {checks:?}"
        );
    }

    #[test]
    fn full_canvas_gfx_distinguishes_fixed_viewport_from_floating_card_pseudoelement() {
        let fixed_clip = Clip {
            id: "full".into(),
            label: "Full".into(),
            start_sec: 0.0,
            duration_sec: 4.0,
            source_in_sec: 0.0,
            transform: Transform2D::default(),
            audio: AudioSettings::default(),
            payload: ClipKind::Gfx {
                gfx: GfxInstance {
                    asset_id: None,
                    props: json!({}),
                    inline_html: Some(
                        r#"<div class="clip" data-start="0" data-duration="4" style="position:fixed; top:0; left:0; right:0; bottom:0;">BG</div>"#
                            .into(),
                    ),
                },
            },
        };
        assert!(checks::is_full_canvas_gfx(&fixed_clip, &[]));

        let floating_card = Clip {
            id: "card".into(),
            label: "Card".into(),
            start_sec: 0.0,
            duration_sec: 4.0,
            source_in_sec: 0.0,
            transform: Transform2D::default(),
            audio: AudioSettings::default(),
            payload: ClipKind::Gfx {
                gfx: GfxInstance {
                    asset_id: None,
                    props: json!({}),
                    inline_html: Some(
                        r#"<style>.card { position: relative; width: 320px; } .card::before { content: ""; position: absolute; inset: 0; }</style><div class="clip card" data-start="0" data-duration="4">Card</div>"#
                            .into(),
                    ),
                },
            },
        };
        assert!(!checks::is_full_canvas_gfx(&floating_card, &[]));

        let saved_asset = SavedCodeAsset {
            id: "full-bg".into(),
            name: "Full BG".into(),
            category: "scenes".into(),
            tags: vec![],
            props_schema: json!({}),
            default_props: json!({}),
            html_source:
                r#"<div class="clip" data-start="0" data-duration="4" style="width:100vw;height:100dvh;">Asset BG</div>"#
                    .into(),
            default_duration_sec: 4.0,
            created_at: Utc::now(),
        };
        let asset_clip = Clip {
            id: "asset-full".into(),
            label: "Asset Full".into(),
            start_sec: 0.0,
            duration_sec: 4.0,
            source_in_sec: 0.0,
            transform: Transform2D::default(),
            audio: AudioSettings::default(),
            payload: ClipKind::Gfx {
                gfx: GfxInstance {
                    asset_id: Some("full-bg".into()),
                    props: json!({}),
                    inline_html: None,
                },
            },
        };
        assert!(checks::is_full_canvas_gfx(&asset_clip, &[saved_asset]));
    }

    #[test]
    fn vision_qa_parser_handles_empty_malformed_blank_entries_and_secret_scrubbing() {
        let (_, empty_checks) = vision::parse_vision_qa_output("   ");
        assert!(
            empty_checks
                .iter()
                .any(|c| c.id == "vision-qa-empty" && c.severity == QaSeverity::Warning)
        );

        let (_, malformed_checks) = vision::parse_vision_qa_output("{truncated");
        assert!(
            malformed_checks
                .iter()
                .any(|c| c.id == "vision-qa-parse-error" && c.severity == QaSeverity::Warning)
        );

        // Blank entries before real entries must be filtered BEFORE `.take(4)`
        let (_, filtered_checks) = vision::parse_vision_qa_output(
            r#"{"summary":"ok","issues":["", "  ", "i1", "i2", "i3", "i4", "i5"],"warnings":[]}"#,
        );
        assert_eq!(filtered_checks.len(), 4);
        assert_eq!(filtered_checks[0].message, "i1");
        assert_eq!(filtered_checks[3].message, "i4");

        let scrubbed =
            vision::scrub_secrets("HTTP 401 Authorization: Bearer sk-or-v1-abc123 key=sk-or-xyz_9");
        assert!(!scrubbed.contains("sk-or-"));
        assert!(scrubbed.contains("Bearer <redacted>"));

        let short_ts = vision::compute_sample_timestamps(0.0, 0.15);
        assert!(!short_ts.is_empty());
        for pair in short_ts.windows(2) {
            assert!(pair[1] > pair[0]);
        }
        assert!(*short_ts.last().unwrap() < 0.15);
    }

    #[tokio::test]
    async fn qa_caps_score_on_hard_issue_marks_cancelled_failed_and_avoids_noop_overlap_fix() {
        // 1. Single hard issue caps score <= 65
        let report_hard = finalize_report(
            vec![QaCheckItem {
                id: "gfx-lint-1".into(),
                category: "gfx".into(),
                severity: QaSeverity::Issue,
                message: "Lint failed".into(),
                time_sec: Some(0.0),
                auto_fixed: false,
            }],
            3,
            0,
            None,
        );
        assert!(!report_hard.passed);
        assert!(report_hard.score <= 65);

        // 2. Cancelled QA returns passed=false with qa-cancelled warning
        let proj = Project::new_default("qa_cancel", AspectPreset::Vertical9x16);
        let pid = proj.id.clone();
        let backend = QaMockBackend {
            engine: RwLock::new(ProjectEngine::new(proj)),
        };
        let cancel = CancellationToken::new();
        cancel.cancel();
        let report_cancelled = run_qa_pass(&backend, None, None, &pid, true, &cancel, None).await;
        assert!(!report_cancelled.passed);
        assert!(
            report_cancelled
                .checks
                .iter()
                .any(|c| c.id == "qa-cancelled" && c.severity == QaSeverity::Warning)
        );

        // 3. No collision-free band emits Warning (not no-op Fixed) and does not mutate
        let proj2 = Project::new_default("qa_blocked_bands", AspectPreset::Vertical9x16);
        let pid2 = proj2.id.clone();
        let mut engine2 = ProjectEngine::new(proj2);
        let meta = CommandMeta::default();
        for (idx, y) in [-0.30, -0.22, 0.0, 0.22, 0.30].into_iter().enumerate() {
            engine2
                .apply(
                    TimelineCommand::PlaceClip {
                        track_id: "t1".into(),
                        clip: Clip {
                            id: format!("cap-{idx}"),
                            label: format!("Cap {idx}"),
                            start_sec: 0.0,
                            duration_sec: 3.0,
                            source_in_sec: 0.0,
                            transform: Transform2D {
                                y,
                                ..Transform2D::default()
                            },
                            audio: AudioSettings::default(),
                            payload: ClipKind::Caption {
                                caption: CaptionBlock {
                                    words: vec![],
                                    preset: CaptionPreset::Minimal,
                                    font_family: "Sora".into(),
                                    highlight_color: "#F7B733".into(),
                                },
                            },
                        },
                    },
                    meta.clone(),
                )
                .unwrap();
        }
        engine2
            .apply(
                TimelineCommand::PlaceGfx {
                    track_id: "g1".into(),
                    clip_id: "gfx-blocked".into(),
                    label: "Badge".into(),
                    start_sec: 0.0,
                    duration_sec: 3.0,
                    gfx: GfxInstance {
                        asset_id: None,
                        props: json!({}),
                        inline_html: Some(
                            r#"<div class="clip" data-start="0" data-duration="3">Badge</div>"#
                                .into(),
                        ),
                    },
                },
                meta,
            )
            .unwrap();
        let backend2 = QaMockBackend {
            engine: RwLock::new(engine2),
        };
        let report_blocked = run_qa_pass(
            &backend2,
            None,
            None,
            &pid2,
            true,
            &CancellationToken::new(),
            None,
        )
        .await;
        assert_eq!(report_blocked.auto_fixes_applied, 0);
        assert!(report_blocked.checks.iter().any(|c| {
            c.id == "overlap-gfx-blocked" && c.severity == QaSeverity::Warning && !c.auto_fixed
        }));
    }

    #[test]
    fn programmatic_qa_samples_settled_mid_scene_timestamps_not_early_entrance() {
        let mut proj = Project::new_default("prog_qa_samples", AspectPreset::Vertical9x16);
        let scenes = [
            (0.0, 3.5),
            (3.5, 4.5),
            (8.0, 4.5),
            (12.5, 4.5),
            (17.0, 4.0),
            (21.0, 3.0),
        ];
        for (i, (start, dur)) in scenes.into_iter().enumerate() {
            proj.tracks[1].clips.push(Clip {
                id: format!("gfx-s{:02}", i + 1),
                label: format!("Scene {}", i + 1),
                start_sec: start,
                duration_sec: dur,
                source_in_sec: 0.0,
                transform: Transform2D::default(),
                audio: AudioSettings::default(),
                payload: ClipKind::Gfx {
                    gfx: GfxInstance {
                        asset_id: None,
                        props: json!({}),
                        inline_html: Some(
                            r#"<div class="clip" data-start="0" data-duration="4">Scene</div>"#
                                .into(),
                        ),
                    },
                },
            });
        }
        let samples = vision::compute_project_sample_timestamps(&proj, 0.0, 24.0);
        assert_eq!(samples.len(), 6);
        // Hook scene [0.0..3.5]: settled at 1.75s (well past 0.4s entrance stagger)
        assert!(samples[0] >= 1.0 && samples[0] <= 2.5, "got {}", samples[0]);
        // Mid scene [12.5..17.0] (index 3): settled mid-scene (~14.75s)
        assert!(
            samples[3] >= 13.5 && samples[3] <= 16.0,
            "got {}",
            samples[3]
        );
        // Final scene [21.0..24.0] (index 5): settled mid-scene (~22.5s)
        assert!(
            samples[5] >= 21.9 && samples[5] <= 23.4,
            "got {}",
            samples[5]
        );
    }
}
