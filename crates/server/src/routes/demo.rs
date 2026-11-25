use std::path::Path;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

use solaris_core_ir::{
    AgentRole, AspectPreset, AudioSettings, CaptionBlock, CaptionPreset, CaptionWord, Clip,
    ClipKind, ColorGrade, CommandMeta, EngineEvent, GfxInstance, MediaAsset,
    MediaIntelligenceManifest, MediaKind, Project, ProjectEngine, SavedCodeAsset, SmartReframeBox,
    TimelineCommand, TranscriptWord, Transform2D,
};
use solaris_media_engine::{DemoMediaFile, ensure_demo_media_pack};

use crate::state::{AppState, map_core_error};

pub const DEMO_PROJECT_ID: &str = "solaris-demo-reel";

#[derive(Deserialize, Default)]
pub struct OpenDemoProjectRequest {
    #[serde(default)]
    pub brand_playbook_id: Option<String>,
}

fn find_asset_html(assets: &[SavedCodeAsset], asset_id: &str) -> Option<String> {
    assets
        .iter()
        .find(|a| a.id == asset_id)
        .map(|a| a.html_source.clone())
}

fn demo_transcript_words() -> Vec<TranscriptWord> {
    vec![
        TranscriptWord {
            word: "THREE".into(),
            start_sec: 0.30,
            end_sec: 1.05,
        },
        TranscriptWord {
            word: "AI".into(),
            start_sec: 1.05,
            end_sec: 1.75,
        },
        TranscriptWord {
            word: "TOOLS".into(),
            start_sec: 1.75,
            end_sec: 3.10,
        },
        TranscriptWord {
            word: "CUT".into(),
            start_sec: 3.50,
            end_sec: 4.35,
        },
        TranscriptWord {
            word: "SILENCES".into(),
            start_sec: 4.35,
            end_sec: 5.45,
        },
        TranscriptWord {
            word: "AUTOMATICALLY".into(),
            start_sec: 5.45,
            end_sec: 6.65,
        },
        TranscriptWord {
            word: "GRADE".into(),
            start_sec: 7.00,
            end_sec: 7.85,
        },
        TranscriptWord {
            word: "LOG".into(),
            start_sec: 7.85,
            end_sec: 8.65,
        },
        TranscriptWord {
            word: "TO".into(),
            start_sec: 8.65,
            end_sec: 9.15,
        },
        TranscriptWord {
            word: "REC.709".into(),
            start_sec: 9.15,
            end_sec: 10.25,
        },
        TranscriptWord {
            word: "SYNC".into(),
            start_sec: 10.60,
            end_sec: 11.40,
        },
        TranscriptWord {
            word: "MOTION".into(),
            start_sec: 11.40,
            end_sec: 12.35,
        },
        TranscriptWord {
            word: "GRAPHICS".into(),
            start_sec: 12.35,
            end_sec: 13.35,
        },
        TranscriptWord {
            word: "SHIP".into(),
            start_sec: 13.70,
            end_sec: 14.35,
        },
        TranscriptWord {
            word: "IN".into(),
            start_sec: 14.35,
            end_sec: 14.85,
        },
        TranscriptWord {
            word: "SECONDS".into(),
            start_sec: 14.85,
            end_sec: 15.75,
        },
    ]
}

fn make_caption_clip(
    id: &str,
    label: &str,
    start_sec: f64,
    duration_sec: f64,
    words: Vec<(&str, f64, f64)>,
) -> Clip {
    Clip {
        id: id.to_string(),
        label: label.to_string(),
        start_sec,
        duration_sec,
        source_in_sec: 0.0,
        transform: Transform2D {
            x: 0.0,
            y: 0.22,
            scale: 1.0,
            rotation_deg: 0.0,
            opacity: 1.0,
            smart_reframe: None,
        },
        audio: AudioSettings::default(),
        payload: ClipKind::Caption {
            caption: CaptionBlock {
                words: words
                    .into_iter()
                    .map(|(text, s, e)| CaptionWord {
                        text: text.to_string(),
                        start_sec: s,
                        end_sec: e,
                    })
                    .collect(),
                preset: CaptionPreset::NeonGlow,
                font_family: "Sora".to_string(),
                highlight_color: "#F7B733".to_string(),
            },
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn make_gfx_clip(
    id: &str,
    label: &str,
    start_sec: f64,
    duration_sec: f64,
    y_offset: f64,
    asset_id: &str,
    props: serde_json::Value,
    assets: &[SavedCodeAsset],
) -> Clip {
    Clip {
        id: id.to_string(),
        label: label.to_string(),
        start_sec,
        duration_sec,
        source_in_sec: 0.0,
        transform: Transform2D {
            x: 0.0,
            y: y_offset,
            scale: 1.0,
            rotation_deg: 0.0,
            opacity: 1.0,
            smart_reframe: None,
        },
        audio: AudioSettings::default(),
        payload: ClipKind::Gfx {
            gfx: GfxInstance {
                asset_id: Some(asset_id.to_string()),
                props,
                inline_html: find_asset_html(assets, asset_id),
            },
        },
    }
}

fn relative_key_for(workspace_dir: &Path, file_path: &Path) -> String {
    file_path
        .strip_prefix(workspace_dir)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| file_path.to_string_lossy().to_string())
}

#[allow(clippy::too_many_arguments)]
fn media_asset_from_demo_file(
    id: &str,
    name: &str,
    role_hint: &str,
    is_log: bool,
    stored_path: String,
    file: &DemoMediaFile,
    transcript: Vec<TranscriptWord>,
    now: chrono::DateTime<Utc>,
) -> MediaAsset {
    let kind = if file.width.is_some() {
        if file.is_video {
            MediaKind::Video
        } else {
            MediaKind::Image
        }
    } else {
        MediaKind::Audio
    };
    MediaAsset {
        id: id.to_string(),
        name: name.to_string(),
        kind,
        role_hint: role_hint.to_string(),
        path: stored_path,
        added_at: now,
        intelligence: MediaIntelligenceManifest {
            analyzed: true,
            duration_sec: file.duration_sec,
            width: file.width,
            height: file.height,
            fps: file.fps,
            codec: Some(
                if kind == MediaKind::Video {
                    "h264"
                } else if kind == MediaKind::Image {
                    "png"
                } else {
                    "pcm_s16le"
                }
                .to_string(),
            ),
            color_space: if is_log {
                Some("S-Log3 / S-Gamut3.Cine".to_string())
            } else if kind != MediaKind::Audio {
                Some("Rec.709".to_string())
            } else {
                None
            },
            is_log_footage: is_log,
            has_audio: file.has_audio,
            waveform_peaks: file.waveform_peaks.clone(),
            silences: Vec::new(),
            beats: file.beats.clone(),
            scene_cuts: Vec::new(),
            default_reframe: if kind != MediaKind::Audio {
                Some(SmartReframeBox {
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0,
                })
            } else {
                None
            },
            transcript,
            integrated_lufs: file.integrated_lufs,
            proxy_path: None,
            analysis_error: None,
        },
    }
}

/// `POST /api/v1/projects/demo`
/// Synthesizes & caches real playable studio media and creates/refreshes the canonical
/// 7-track Demo Studio project (`solaris-demo-reel`) without polluting Recent projects.
pub async fn open_demo_project(
    State(state): State<AppState>,
    body: Option<Json<OpenDemoProjectRequest>>,
) -> Result<(StatusCode, Json<Project>), (StatusCode, String)> {
    let requested_brand = body
        .and_then(|Json(req)| req.brand_playbook_id)
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty());
    let brand_id = {
        let brands = state.inner.brands.read().await;
        match requested_brand {
            Some(id) if brands.contains_key(&id) => id,
            _ => "solaris-default".to_string(),
        }
    };

    // Serialize demo media synthesis & project reset so concurrent requests never spawn parallel FFmpeg runs
    let _demo_lock = state.inner.demo_pack_lock.lock().await;

    // Cancel any active swarm run targeting the demo project before resetting it
    let active_run_ids: Vec<String> = {
        let runs = state.inner.swarm_runs.read().await;
        runs.values()
            .filter(|r| r.project_id == DEMO_PROJECT_ID)
            .map(|r| r.run_id.clone())
            .collect()
    };
    if !active_run_ids.is_empty() {
        let cancels = state.inner.swarm_cancels.read().await;
        for run_id in active_run_ids {
            if let Some(tok) = cancels.get(&run_id) {
                tok.cancel();
            }
        }
    }

    let workspace_dir = state.inner.workspace_dir.clone();
    let toolchain = state.inner.toolchain.clone();
    let storage = state.inner.storage.clone();

    let (pack, paths) = tokio::task::spawn_blocking(move || {
        let pack = ensure_demo_media_pack(&workspace_dir, &toolchain)
            .map_err(|e| format!("failed to prepare demo media: {e}"))?;
        let sync_one = |f: &DemoMediaFile| -> Result<String, String> {
            let rel = relative_key_for(&workspace_dir, &f.path);
            storage
                .sync_file(&rel, &f.path)
                .map_err(|e| format!("failed to sync demo media '{rel}': {e}"))
        };
        let p_aroll = sync_one(&pack.a_roll)?;
        let p_broll1 = sync_one(&pack.b_roll_workflow)?;
        let p_broll2 = sync_one(&pack.b_roll_grade)?;
        let p_vo = sync_one(&pack.voice_stem)?;
        let p_music = sync_one(&pack.music_beat)?;
        Ok::<_, String>((pack, (p_aroll, p_broll1, p_broll2, p_vo, p_music)))
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let assets: Vec<SavedCodeAsset> = state.inner.assets.read().await.values().cloned().collect();
    let now = Utc::now();

    let mut project = Project::new_default("solaris_studio_demo", AspectPreset::Vertical9x16);
    project.id = DEMO_PROJECT_ID.to_string();
    project.brand_playbook_id = brand_id;
    project.prompt =
        "Three AI Tools Reel — 9:16 Showcase with Log Grade, GSAP Overlays, Captions & Ducked Mix"
            .to_string();

    let mut engine = ProjectEngine::new(project);

    let a_roll_name = if pack.a_roll.is_video {
        "founder_studio_slog3.mp4"
    } else {
        "founder_studio_slog3.png"
    };
    let broll1_name = if pack.b_roll_workflow.is_video {
        "workflow_broll_solar.mp4"
    } else {
        "workflow_broll_solar.png"
    };
    let broll2_name = if pack.b_roll_grade.is_video {
        "studio_grade_broll.mp4"
    } else {
        "studio_grade_broll.png"
    };

    let demo_ops: Vec<(CommandMeta, TimelineCommand)> = vec![
        // 1. Director imports 5 real playable media assets
        (
            CommandMeta {
                agent_id: "director-01".into(),
                role: AgentRole::Director,
                reason: Some("import demo studio a-roll".into()),
            },
            TimelineCommand::AddMedia {
                asset: media_asset_from_demo_file(
                    "demo-m1",
                    a_roll_name,
                    "a_roll",
                    true,
                    paths.0,
                    &pack.a_roll,
                    demo_transcript_words(),
                    now,
                ),
            },
        ),
        (
            CommandMeta {
                agent_id: "director-01".into(),
                role: AgentRole::Director,
                reason: Some("import workflow b-roll".into()),
            },
            TimelineCommand::AddMedia {
                asset: media_asset_from_demo_file(
                    "demo-m2",
                    broll1_name,
                    "b_roll",
                    false,
                    paths.1,
                    &pack.b_roll_workflow,
                    Vec::new(),
                    now,
                ),
            },
        ),
        (
            CommandMeta {
                agent_id: "director-01".into(),
                role: AgentRole::Director,
                reason: Some("import solar horizon b-roll".into()),
            },
            TimelineCommand::AddMedia {
                asset: media_asset_from_demo_file(
                    "demo-m3",
                    broll2_name,
                    "b_roll",
                    false,
                    paths.2,
                    &pack.b_roll_grade,
                    Vec::new(),
                    now,
                ),
            },
        ),
        (
            CommandMeta {
                agent_id: "director-01".into(),
                role: AgentRole::Director,
                reason: Some("import narration cadence stem".into()),
            },
            TimelineCommand::AddMedia {
                asset: media_asset_from_demo_file(
                    "demo-vo",
                    "narrator_cadence_vo.wav",
                    "voice",
                    false,
                    paths.3,
                    &pack.voice_stem,
                    demo_transcript_words(),
                    now,
                ),
            },
        ),
        (
            CommandMeta {
                agent_id: "director-01".into(),
                role: AgentRole::Director,
                reason: Some("import 118 bpm beat".into()),
            },
            TimelineCommand::AddMedia {
                asset: media_asset_from_demo_file(
                    "demo-music",
                    "solaris_beat_118bpm.wav",
                    "music",
                    false,
                    paths.4,
                    &pack.music_beat,
                    Vec::new(),
                    now,
                ),
            },
        ),
        // 2. Razor cuts full 16.0s V1 A-Roll spine with rhythmic punch-ins
        (
            CommandMeta {
                agent_id: "razor-01".into(),
                role: AgentRole::Razor,
                reason: Some("hook punch-in cut".into()),
            },
            TimelineCommand::PlaceClip {
                track_id: "v1".into(),
                clip: Clip {
                    id: "demo-aroll-1".into(),
                    label: "Hook · Punch-In 1.08×".into(),
                    start_sec: 0.0,
                    duration_sec: 5.2,
                    source_in_sec: 0.0,
                    transform: Transform2D {
                        scale: 1.08,
                        ..Transform2D::default()
                    },
                    audio: AudioSettings {
                        gain_db: -99.0,
                        ..AudioSettings::default()
                    },
                    payload: ClipKind::Media {
                        media_id: "demo-m1".into(),
                        ken_burns: !pack.a_roll.is_video,
                    },
                },
            },
        ),
        (
            CommandMeta {
                agent_id: "razor-01".into(),
                role: AgentRole::Razor,
                reason: Some("core wide framing cut".into()),
            },
            TimelineCommand::PlaceClip {
                track_id: "v1".into(),
                clip: Clip {
                    id: "demo-aroll-2".into(),
                    label: "Core · Studio Wide".into(),
                    start_sec: 5.2,
                    duration_sec: 5.6,
                    source_in_sec: 5.2,
                    transform: Transform2D::default(),
                    audio: AudioSettings {
                        gain_db: -99.0,
                        ..AudioSettings::default()
                    },
                    payload: ClipKind::Media {
                        media_id: "demo-m1".into(),
                        ken_burns: !pack.a_roll.is_video,
                    },
                },
            },
        ),
        (
            CommandMeta {
                agent_id: "razor-01".into(),
                role: AgentRole::Razor,
                reason: Some("outro push-in cut".into()),
            },
            TimelineCommand::PlaceClip {
                track_id: "v1".into(),
                clip: Clip {
                    id: "demo-aroll-3".into(),
                    label: "Outro · Push-In 1.06×".into(),
                    start_sec: 10.8,
                    duration_sec: 5.2,
                    source_in_sec: 10.8,
                    transform: Transform2D {
                        scale: 1.06,
                        ..Transform2D::default()
                    },
                    audio: AudioSettings {
                        gain_db: -99.0,
                        ..AudioSettings::default()
                    },
                    payload: ClipKind::Media {
                        media_id: "demo-m1".into(),
                        ken_burns: !pack.a_roll.is_video,
                    },
                },
            },
        ),
        // 3. B-Roll specialist places two Ken Burns visual inserts on V2
        (
            CommandMeta {
                agent_id: "broll-01".into(),
                role: AgentRole::BRoll,
                reason: Some("orbital workflow cutaway".into()),
            },
            TimelineCommand::PlaceClip {
                track_id: "v2".into(),
                clip: Clip {
                    id: "demo-broll-1".into(),
                    label: "Orbital Workflow B-Roll".into(),
                    start_sec: 3.6,
                    duration_sec: 3.4,
                    source_in_sec: 0.0,
                    transform: Transform2D::default(),
                    audio: AudioSettings {
                        gain_db: -99.0,
                        ..AudioSettings::default()
                    },
                    payload: ClipKind::Media {
                        media_id: "demo-m2".into(),
                        ken_burns: true,
                    },
                },
            },
        ),
        (
            CommandMeta {
                agent_id: "broll-01".into(),
                role: AgentRole::BRoll,
                reason: Some("solar horizon color cutaway".into()),
            },
            TimelineCommand::PlaceClip {
                track_id: "v2".into(),
                clip: Clip {
                    id: "demo-broll-2".into(),
                    label: "Solar Horizon B-Roll".into(),
                    start_sec: 10.6,
                    duration_sec: 3.0,
                    source_in_sec: 0.0,
                    transform: Transform2D::default(),
                    audio: AudioSettings {
                        gain_db: -99.0,
                        ..AudioSettings::default()
                    },
                    payload: ClipKind::Media {
                        media_id: "demo-m3".into(),
                        ken_burns: true,
                    },
                },
            },
        ),
        // 4. Grade specialist adds FX adjustment layer clip for Rec.709 S-Log3 recovery
        (
            CommandMeta {
                agent_id: "grade-01".into(),
                role: AgentRole::Grade,
                reason: Some("S-Log3 to Rec.709 Solar Look".into()),
            },
            TimelineCommand::PlaceClip {
                track_id: "fx".into(),
                clip: Clip {
                    id: "demo-fx-1".into(),
                    label: "Solar Rec.709 Look".into(),
                    start_sec: 0.0,
                    duration_sec: 16.0,
                    source_in_sec: 0.0,
                    transform: Transform2D::default(),
                    audio: AudioSettings::default(),
                    payload: ClipKind::Grade {
                        grade: ColorGrade {
                            contrast: 1.14,
                            saturation: 1.12,
                            temp: 0.06,
                            exposure: 0.04,
                            ..ColorGrade::default()
                        },
                    },
                },
            },
        ),
        // 5. GFX specialist places 4 animated GSAP cards in the upper-mid zone (y = -0.18, clear of T1 captions)
        (
            CommandMeta {
                agent_id: "gfx-01".into(),
                role: AgentRole::Gfx,
                reason: Some("opening chapter title".into()),
            },
            TimelineCommand::PlaceClip {
                track_id: "g1".into(),
                clip: make_gfx_clip(
                    "demo-gfx-1",
                    "Chapter · 3 AI TOOLS",
                    0.2,
                    3.0,
                    -0.18,
                    "chapter-title-solar",
                    json!({ "eyebrow": "SOLARIS STUDIO", "headline": "3 AI TOOLS" }),
                    &assets,
                ),
            },
        ),
        (
            CommandMeta {
                agent_id: "gfx-01".into(),
                role: AgentRole::Gfx,
                reason: Some("animated stat card".into()),
            },
            TimelineCommand::PlaceClip {
                track_id: "g1".into(),
                clip: make_gfx_clip(
                    "demo-gfx-2",
                    "Stat Card · 10 HRS",
                    3.8,
                    3.0,
                    -0.18,
                    "stat-card-solar",
                    json!({ "value": "10 HRS", "label": "SAVED EVERY WEEK" }),
                    &assets,
                ),
            },
        ),
        (
            CommandMeta {
                agent_id: "gfx-01".into(),
                role: AgentRole::Gfx,
                reason: Some("animated progress pill".into()),
            },
            TimelineCommand::PlaceClip {
                track_id: "g1".into(),
                clip: make_gfx_clip(
                    "demo-gfx-3",
                    "Progress · Pipeline",
                    7.4,
                    3.0,
                    -0.18,
                    "progress-bar-solar",
                    json!({ "label": "PIPELINE · 02 / 03", "percent": 78 }),
                    &assets,
                ),
            },
        ),
        (
            CommandMeta {
                agent_id: "gfx-01".into(),
                role: AgentRole::Gfx,
                reason: Some("closing social callout".into()),
            },
            TimelineCommand::PlaceClip {
                track_id: "g1".into(),
                clip: make_gfx_clip(
                    "demo-gfx-4",
                    "Comment · @solaris",
                    11.8,
                    3.8,
                    -0.18,
                    "comment-bubble-solar",
                    json!({ "author": "@solaris.studio", "comment": "Cut, graded & mixed in 40 seconds." }),
                    &assets,
                ),
            },
        ),
        // 6. Captions specialist places 5 word-synced Neon Glow bursts across the full 16s reel
        (
            CommandMeta {
                agent_id: "caps-01".into(),
                role: AgentRole::Caps,
                reason: Some("hook caption burst".into()),
            },
            TimelineCommand::PlaceClip {
                track_id: "t1".into(),
                clip: make_caption_clip(
                    "demo-cap-1",
                    "THREE AI TOOLS",
                    0.3,
                    2.9,
                    vec![
                        ("THREE", 0.30, 1.05),
                        ("AI", 1.05, 1.75),
                        ("TOOLS", 1.75, 3.10),
                    ],
                ),
            },
        ),
        (
            CommandMeta {
                agent_id: "caps-01".into(),
                role: AgentRole::Caps,
                reason: Some("silence cut caption burst".into()),
            },
            TimelineCommand::PlaceClip {
                track_id: "t1".into(),
                clip: make_caption_clip(
                    "demo-cap-2",
                    "CUT SILENCES AUTOMATICALLY",
                    3.5,
                    3.2,
                    vec![
                        ("CUT", 3.50, 4.35),
                        ("SILENCES", 4.35, 5.45),
                        ("AUTOMATICALLY", 5.45, 6.65),
                    ],
                ),
            },
        ),
        (
            CommandMeta {
                agent_id: "caps-01".into(),
                role: AgentRole::Caps,
                reason: Some("color grade caption burst".into()),
            },
            TimelineCommand::PlaceClip {
                track_id: "t1".into(),
                clip: make_caption_clip(
                    "demo-cap-3",
                    "GRADE LOG TO REC.709",
                    7.0,
                    3.3,
                    vec![
                        ("GRADE", 7.00, 7.85),
                        ("LOG", 7.85, 8.65),
                        ("TO", 8.65, 9.15),
                        ("REC.709", 9.15, 10.25),
                    ],
                ),
            },
        ),
        (
            CommandMeta {
                agent_id: "caps-01".into(),
                role: AgentRole::Caps,
                reason: Some("motion graphics caption burst".into()),
            },
            TimelineCommand::PlaceClip {
                track_id: "t1".into(),
                clip: make_caption_clip(
                    "demo-cap-4",
                    "SYNC MOTION GRAPHICS",
                    10.6,
                    2.8,
                    vec![
                        ("SYNC", 10.60, 11.40),
                        ("MOTION", 11.40, 12.35),
                        ("GRAPHICS", 12.35, 13.35),
                    ],
                ),
            },
        ),
        (
            CommandMeta {
                agent_id: "caps-01".into(),
                role: AgentRole::Caps,
                reason: Some("outro caption burst".into()),
            },
            TimelineCommand::PlaceClip {
                track_id: "t1".into(),
                clip: make_caption_clip(
                    "demo-cap-5",
                    "SHIP IN SECONDS",
                    13.7,
                    2.1,
                    vec![
                        ("SHIP", 13.70, 14.35),
                        ("IN", 14.35, 14.85),
                        ("SECONDS", 14.85, 15.75),
                    ],
                ),
            },
        ),
        // 7. Mix specialist places A1 Voice Cadence + A2 Ducked 118 BPM Beat
        (
            CommandMeta {
                agent_id: "mix-01".into(),
                role: AgentRole::Mix,
                reason: Some("place voice stem at -14 LUFS".into()),
            },
            TimelineCommand::PlaceClip {
                track_id: "a1".into(),
                clip: Clip {
                    id: "demo-voice-1".into(),
                    label: "narrator_cadence_vo.wav · -14 LUFS".into(),
                    start_sec: 0.2,
                    duration_sec: 15.6,
                    source_in_sec: 0.2,
                    transform: Transform2D::default(),
                    audio: AudioSettings {
                        gain_db: -2.0,
                        ducking_enabled: false,
                        duck_target_db: -18.0,
                        fade_in_sec: 0.1,
                        fade_out_sec: 0.3,
                    },
                    payload: ClipKind::Media {
                        media_id: "demo-vo".into(),
                        ken_burns: false,
                    },
                },
            },
        ),
        (
            CommandMeta {
                agent_id: "mix-01".into(),
                role: AgentRole::Mix,
                reason: Some("place ducked 118 BPM beat".into()),
            },
            TimelineCommand::PlaceClip {
                track_id: "a2".into(),
                clip: Clip {
                    id: "demo-music-1".into(),
                    label: "solaris_beat_118bpm.wav · ducked".into(),
                    start_sec: 0.0,
                    duration_sec: 16.0,
                    source_in_sec: 0.0,
                    transform: Transform2D::default(),
                    audio: AudioSettings {
                        gain_db: -9.0,
                        ducking_enabled: true,
                        duck_target_db: -18.0,
                        fade_in_sec: 0.25,
                        fade_out_sec: 1.0,
                    },
                    payload: ClipKind::Media {
                        media_id: "demo-music".into(),
                        ken_burns: false,
                    },
                },
            },
        ),
    ];

    for (meta, cmd) in demo_ops {
        engine.apply(cmd, meta).map_err(map_core_error)?;
    }

    let built = engine.project().clone();
    {
        let mut guard = state.inner.projects.write().await;
        let replaced = guard.contains_key(&built.id);
        guard.insert(built.id.clone(), engine);
        state.force_persist_project(&built).await;
        if replaced {
            state.emit(EngineEvent::ProjectReset {
                project_id: built.id.clone(),
                revision: built.revision,
                reason: "Demo Studio project reset".to_string(),
                project: Box::new(built.clone()),
            });
        }
    }

    Ok((StatusCode::CREATED, Json(built)))
}
