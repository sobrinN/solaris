use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::json;
use solaris_core_ir::{
    AgentRole, AudioSettings, BrandPlaybook, Clip, ClipKind, CommandMeta, MediaAsset, MediaKind,
    Project, SavedCodeAsset, TimelineCommand, Transform2D,
};
use tokio_util::sync::CancellationToken;

use crate::backend::SolarisBackend;
use crate::error::AgentError;
use crate::swarm::is_voice_hint;
use crate::tools::execute_tool;

const BREATH_PAD_SEC: f64 = 0.12;
const MIN_SILENCE_CUT_SEC: f64 = 0.25;

fn is_voice_asset(asset: &MediaAsset) -> bool {
    if asset.kind != MediaKind::Audio {
        return false;
    }
    if is_voice_hint(&asset.role_hint) {
        return true;
    }
    asset.role_hint.eq_ignore_ascii_case("audio") && !asset.intelligence.transcript.is_empty()
}

fn compute_speech_segments(asset: &MediaAsset, max_total_dur: f64) -> Vec<(f64, f64)> {
    let dur =
        if asset.intelligence.duration_sec.is_finite() && asset.intelligence.duration_sec > 0.2 {
            asset.intelligence.duration_sec.min(max_total_dur)
        } else {
            4.0_f64.min(max_total_dur)
        };

    // Sort and merge overlapping/unsorted silences before computing speech windows
    let mut silences: Vec<(f64, f64)> = asset
        .intelligence
        .silences
        .iter()
        .filter(|s| s.start_sec.is_finite() && s.end_sec.is_finite() && s.end_sec > s.start_sec)
        .map(|s| (s.start_sec.max(0.0), s.end_sec.min(dur)))
        .filter(|(s, e)| *e - *s > MIN_SILENCE_CUT_SEC && *s < dur)
        .collect();
    silences.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (s, e) in silences {
        if let Some(last) = merged.last_mut()
            && s <= last.1
        {
            last.1 = last.1.max(e);
        } else {
            merged.push((s, e));
        }
    }

    let mut cursor = 0.0_f64;
    let mut segs = Vec::new();
    for (sil_start, sil_end) in merged {
        let seg_end = (sil_start + BREATH_PAD_SEC).min(dur);
        if seg_end - cursor > 0.25 {
            segs.push((cursor, seg_end - cursor));
        }
        cursor = (sil_end - BREATH_PAD_SEC).max(seg_end).min(dur);
    }
    if dur - cursor > 0.25 {
        segs.push((cursor, dur - cursor));
    }
    if segs.is_empty() {
        segs.push((0.0, dur));
    }
    segs
}

fn spine_duration_sec(project: &Project) -> f64 {
    project
        .tracks
        .iter()
        .filter(|t| t.id == "v1" || t.id == "a1")
        .flat_map(|t| t.clips.iter())
        .map(|c| c.start_sec + c.duration_sec)
        .fold(0.0_f64, f64::max)
}

pub(crate) async fn execute_deterministic_spine(
    backend: &Arc<dyn SolarisBackend>,
    project: &Project,
    cancel: &CancellationToken,
    ops_counter: &Arc<AtomicU32>,
) -> Result<(), AgentError> {
    if cancel.is_cancelled() {
        return Err(AgentError::Cancelled);
    }

    let meta = CommandMeta {
        agent_id: "razor-01".to_string(),
        role: AgentRole::Razor,
        reason: Some("local spine edit".to_string()),
    };

    let a1_empty = project
        .tracks
        .iter()
        .find(|t| t.id == "a1")
        .is_none_or(|t| t.clips.is_empty());

    let mut voice_total_dur = 0.0_f64;
    if a1_empty {
        let voice_assets: Vec<_> = project
            .media_pool
            .iter()
            .filter(|m| is_voice_asset(m))
            .collect();
        let mut a1_cursor = 0.0_f64;
        let mut a1_idx = 0usize;
        for asset in voice_assets.iter().take(4) {
            for (src_in, seg_dur) in compute_speech_segments(asset, 24.0) {
                if cancel.is_cancelled() {
                    return Err(AgentError::Cancelled);
                }
                let clip = Clip {
                    id: format!("swarm-a1-{a1_idx}-{}", uuid::Uuid::new_v4().simple()),
                    label: format!("{} · VO {}", asset.name, a1_idx + 1),
                    start_sec: a1_cursor,
                    duration_sec: seg_dur,
                    source_in_sec: src_in,
                    transform: Transform2D::default(),
                    audio: AudioSettings::default(),
                    payload: ClipKind::Media {
                        media_id: asset.id.clone(),
                        ken_burns: false,
                    },
                };
                if backend
                    .apply_op(
                        &project.id,
                        TimelineCommand::PlaceClip {
                            track_id: "a1".to_string(),
                            clip,
                        },
                        meta.clone(),
                    )
                    .await
                    .is_ok()
                {
                    ops_counter.fetch_add(1, Ordering::Relaxed);
                    a1_cursor += seg_dur;
                    a1_idx += 1;
                }
            }
        }
        voice_total_dur = a1_cursor;
    }

    let v1_empty = project
        .tracks
        .iter()
        .find(|t| t.id == "v1")
        .is_none_or(|t| t.clips.is_empty());

    if v1_empty {
        let mut timeline_cursor = 0.0_f64;
        // Prefer explicit A-roll videos; fall back to any video (b-roll-only projects); fall back to images (images-only projects).
        let primary_videos: Vec<_> = project
            .media_pool
            .iter()
            .filter(|m| m.kind == MediaKind::Video && m.role_hint != "b_roll")
            .collect();
        let video_candidates: Vec<_> = if !primary_videos.is_empty() {
            primary_videos
        } else {
            project
                .media_pool
                .iter()
                .filter(|m| m.kind == MediaKind::Video)
                .collect()
        };

        let mut cut_idx = 0usize;
        if !video_candidates.is_empty() {
            for asset in video_candidates.iter().take(4) {
                for (src_in, seg_dur) in compute_speech_segments(asset, 14.0) {
                    if cancel.is_cancelled() {
                        return Err(AgentError::Cancelled);
                    }
                    let clip = Clip {
                        id: format!("swarm-v1-{cut_idx}-{}", uuid::Uuid::new_v4().simple()),
                        label: format!("{} · Cut {}", asset.name, cut_idx + 1),
                        start_sec: timeline_cursor,
                        duration_sec: seg_dur,
                        source_in_sec: src_in,
                        transform: Transform2D {
                            scale: if cut_idx.is_multiple_of(2) { 1.08 } else { 1.0 },
                            ..Transform2D::default()
                        },
                        audio: AudioSettings::default(),
                        payload: ClipKind::Media {
                            media_id: asset.id.clone(),
                            ken_burns: false,
                        },
                    };
                    if backend
                        .apply_op(
                            &project.id,
                            TimelineCommand::PlaceClip {
                                track_id: "v1".to_string(),
                                clip,
                            },
                            meta.clone(),
                        )
                        .await
                        .is_ok()
                    {
                        ops_counter.fetch_add(1, Ordering::Relaxed);
                        timeline_cursor += seg_dur;
                        cut_idx += 1;
                    }
                }
            }
        } else {
            // Images-only (or Faceless Explainer with voiceover + stills): fill V1 with Ken Burns stills
            let images: Vec<_> = project
                .media_pool
                .iter()
                .filter(|m| m.kind == MediaKind::Image)
                .take(8)
                .collect();
            if !images.is_empty() {
                let target_total = if voice_total_dur > 0.5 {
                    voice_total_dur
                } else {
                    (images.len() as f64) * 3.0
                };
                let per_img = (target_total / images.len() as f64).clamp(1.2, 6.0);
                for img in images {
                    if cancel.is_cancelled() {
                        return Err(AgentError::Cancelled);
                    }
                    let clip = Clip {
                        id: format!("swarm-v1-{cut_idx}-{}", uuid::Uuid::new_v4().simple()),
                        label: format!("{} · Scene {}", img.name, cut_idx + 1),
                        start_sec: timeline_cursor,
                        duration_sec: per_img,
                        source_in_sec: 0.0,
                        transform: Transform2D::default(),
                        audio: AudioSettings {
                            gain_db: -99.0,
                            ..AudioSettings::default()
                        },
                        payload: ClipKind::Media {
                            media_id: img.id.clone(),
                            ken_burns: true,
                        },
                    };
                    if backend
                        .apply_op(
                            &project.id,
                            TimelineCommand::PlaceClip {
                                track_id: "v1".to_string(),
                                clip,
                            },
                            meta.clone(),
                        )
                        .await
                        .is_ok()
                    {
                        ops_counter.fetch_add(1, Ordering::Relaxed);
                        timeline_cursor += per_img;
                        cut_idx += 1;
                    }
                }
            }
        }
    }

    Ok(())
}

pub(crate) async fn execute_deterministic_specialist(
    backend: &Arc<dyn SolarisBackend>,
    project: &Project,
    brand: &BrandPlaybook,
    assets: &[SavedCodeAsset],
    role: AgentRole,
    cancel: &CancellationToken,
    ops_counter: &Arc<AtomicU32>,
) -> u32 {
    if cancel.is_cancelled() {
        return 0;
    }

    let spine_dur = spine_duration_sec(project);
    let mut committed: u32 = 0;

    match role {
        AgentRole::BRoll => {
            let v2_empty = project
                .tracks
                .iter()
                .find(|t| t.id == "v2")
                .is_none_or(|t| t.clips.is_empty());
            if !v2_empty || spine_dur <= 0.4 {
                return 0;
            }
            let meta = CommandMeta {
                agent_id: "broll-01".to_string(),
                role,
                reason: Some("local b-roll insert".to_string()),
            };
            let v1_media_ids: Vec<&str> = project
                .tracks
                .iter()
                .find(|t| t.id == "v1")
                .map(|t| {
                    t.clips
                        .iter()
                        .filter_map(|c| match &c.payload {
                            ClipKind::Media { media_id, .. } => Some(media_id.as_str()),
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default();

            let broll = project
                .media_pool
                .iter()
                .find(|m| {
                    (m.role_hint == "b_roll" || m.kind == MediaKind::Image)
                        && !v1_media_ids.contains(&m.id.as_str())
                })
                .or_else(|| {
                    project
                        .media_pool
                        .iter()
                        .find(|m| m.role_hint == "b_roll" && m.kind == MediaKind::Video)
                });

            if let Some(broll) = broll {
                let start_sec = if spine_dur >= 3.5 {
                    2.5_f64.min((spine_dur - 0.8).max(0.0))
                } else {
                    (spine_dur * 0.35).min((spine_dur - 0.5).max(0.0))
                };
                let max_fit = (spine_dur - start_sec).max(0.4);
                let natural = if broll.intelligence.duration_sec > 0.2 {
                    broll.intelligence.duration_sec.min(3.5)
                } else {
                    3.0
                };
                let dur = natural.min(max_fit);
                if dur < 0.25 {
                    return 0;
                }
                let clip = Clip {
                    id: format!("swarm-v2-{}", uuid::Uuid::new_v4().simple()),
                    label: format!("{} · Insert", broll.name),
                    start_sec,
                    duration_sec: dur,
                    source_in_sec: 0.0,
                    transform: Transform2D::default(),
                    audio: AudioSettings {
                        gain_db: -99.0,
                        ..AudioSettings::default()
                    },
                    payload: ClipKind::Media {
                        media_id: broll.id.clone(),
                        ken_burns: broll.kind == MediaKind::Image,
                    },
                };
                if backend
                    .apply_op(
                        &project.id,
                        TimelineCommand::PlaceClip {
                            track_id: "v2".to_string(),
                            clip,
                        },
                        meta,
                    )
                    .await
                    .is_ok()
                {
                    ops_counter.fetch_add(1, Ordering::Relaxed);
                    committed += 1;
                }
            }
        }
        AgentRole::Grade => {
            let meta = CommandMeta {
                agent_id: "grade-01".to_string(),
                role,
                reason: Some("local color grade".to_string()),
            };
            let has_log = project
                .media_pool
                .iter()
                .any(|m| m.intelligence.is_log_footage);
            let mut grade = project.active_grade.clone();
            if has_log {
                grade.contrast = 1.18;
                grade.saturation = 1.15;
            }
            if let Some(lut) = &brand.default_lut {
                grade.lut_path = Some(lut.clone());
                grade.lut_intensity = 0.78;
            }
            // Skip committing no-op color grade op when nothing changed
            if grade == project.active_grade {
                return 0;
            }
            if backend
                .apply_op(
                    &project.id,
                    TimelineCommand::SetColorGrade {
                        grade,
                        track_id: None,
                        clip_id: None,
                    },
                    meta,
                )
                .await
                .is_ok()
            {
                ops_counter.fetch_add(1, Ordering::Relaxed);
                committed += 1;
            }
        }
        AgentRole::Caps => {
            let has_transcript = project
                .media_pool
                .iter()
                .any(|m| !m.intelligence.transcript.is_empty());
            if !has_transcript {
                return 0;
            }
            let meta = CommandMeta {
                agent_id: "caps-01".to_string(),
                role,
                reason: Some("local captions".to_string()),
            };
            if execute_tool(
                backend.as_ref(),
                "captions.generate_from_transcript",
                json!({ "project_id": project.id, "preset": "neon_glow", "words_per_burst": 3 }),
                meta,
            )
            .await
            .is_ok()
            {
                ops_counter.fetch_add(1, Ordering::Relaxed);
                committed += 1;
            }
        }
        AgentRole::Gfx => {
            let g1_empty = project
                .tracks
                .iter()
                .find(|t| t.id == "g1")
                .is_none_or(|t| t.clips.is_empty());
            if !g1_empty || spine_dur <= 0.4 {
                return 0;
            }
            let meta = CommandMeta {
                agent_id: "gfx-01".to_string(),
                role,
                reason: Some("local gfx card".to_string()),
            };
            if let Some(asset) = assets.first() {
                let start_sec = if spine_dur >= 2.0 { 0.6 } else { 0.1 };
                let dur = asset
                    .default_duration_sec
                    .min((spine_dur - start_sec).max(0.5))
                    .max(0.4);
                if execute_tool(
                    backend.as_ref(),
                    "gfx.place",
                    json!({
                        "project_id": project.id,
                        "clip_id": format!("swarm-g1-{}", uuid::Uuid::new_v4().simple()),
                        "label": asset.name,
                        "start_sec": start_sec,
                        "duration_sec": dur,
                        "asset_id": asset.id,
                        "props": asset.default_props,
                    }),
                    meta,
                )
                .await
                .is_ok()
                {
                    ops_counter.fetch_add(1, Ordering::Relaxed);
                    committed += 1;
                }
            }
        }
        AgentRole::Mix => {
            let meta = CommandMeta {
                agent_id: "mix-01".to_string(),
                role,
                reason: Some("local audio mix".to_string()),
            };
            let a2_empty = project
                .tracks
                .iter()
                .find(|t| t.id == "a2")
                .is_none_or(|t| t.clips.is_empty());

            // Never place voiceover (or transcribed generic audio) assets onto A2 music ducked track
            if a2_empty
                && let Some(music) = project
                    .media_pool
                    .iter()
                    .find(|m| m.kind == MediaKind::Audio && !is_voice_asset(m))
            {
                let natural_dur = if music.intelligence.duration_sec > 0.5 {
                    music.intelligence.duration_sec
                } else {
                    10.0
                };
                let dur = if spine_dur > 0.25 {
                    natural_dur.min(spine_dur)
                } else {
                    natural_dur.min(18.0)
                };
                if dur > 0.2 {
                    let fade_in = 0.25_f64.min(dur * 0.25);
                    let fade_out = 0.8_f64.min((dur - fade_in).max(0.0));
                    let clip = Clip {
                        id: format!("swarm-a2-{}", uuid::Uuid::new_v4().simple()),
                        label: format!("{} · Ducked", music.name),
                        start_sec: 0.0,
                        duration_sec: dur,
                        source_in_sec: 0.0,
                        transform: Transform2D::default(),
                        audio: AudioSettings {
                            gain_db: -15.0,
                            ducking_enabled: true,
                            duck_target_db: -18.0,
                            fade_in_sec: fade_in,
                            fade_out_sec: fade_out,
                        },
                        payload: ClipKind::Media {
                            media_id: music.id.clone(),
                            ken_burns: false,
                        },
                    };
                    if backend
                        .apply_op(
                            &project.id,
                            TimelineCommand::PlaceClip {
                                track_id: "a2".to_string(),
                                clip,
                            },
                            meta.clone(),
                        )
                        .await
                        .is_ok()
                    {
                        ops_counter.fetch_add(1, Ordering::Relaxed);
                        committed += 1;
                    }
                }
            }
            if cancel.is_cancelled() {
                return committed;
            }
            let has_lufs_speech = project
                .tracks
                .iter()
                .filter(|t| (t.id == "a1" || t.id == "v1") && !t.muted)
                .flat_map(|t| t.clips.iter())
                .any(|c| match &c.payload {
                    ClipKind::Media { media_id, .. } => project
                        .media_pool
                        .iter()
                        .find(|m| m.id == *media_id)
                        .is_some_and(|m| m.intelligence.integrated_lufs.is_some()),
                    _ => false,
                });
            if has_lufs_speech
                && execute_tool(
                    backend.as_ref(),
                    "audio.normalize_lufs",
                    json!({ "project_id": project.id, "target_lufs": -14.0 }),
                    meta,
                )
                .await
                .is_ok()
            {
                ops_counter.fetch_add(1, Ordering::Relaxed);
                committed += 1;
            }
        }
        _ => {}
    }

    committed
}
