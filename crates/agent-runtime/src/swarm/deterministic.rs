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
use crate::swarm::speech::narrated_transcript_present;
use crate::swarm::{DirectorPlan, is_voice_hint};
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

#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_deterministic_specialist(
    backend: &Arc<dyn SolarisBackend>,
    project: &Project,
    brand: &BrandPlaybook,
    assets: &[SavedCodeAsset],
    plan: &DirectorPlan,
    prompt: &str,
    role: AgentRole,
    cancel: &CancellationToken,
    ops_counter: &Arc<AtomicU32>,
) -> u32 {
    if cancel.is_cancelled() {
        return 0;
    }

    let spine_dur = spine_duration_sec(project);
    let has_visual_media = project
        .media_pool
        .iter()
        .any(|m| matches!(m.kind, MediaKind::Video | MediaKind::Image));
    let has_a1_clips = project
        .tracks
        .iter()
        .find(|t| t.id == "a1")
        .is_some_and(|t| !t.clips.is_empty());
    let target_dur = if !has_visual_media && has_a1_clips && spine_dur > 0.0 {
        spine_dur.clamp(1.0, 120.0)
    } else if plan.target_duration_sec.is_finite() && plan.target_duration_sec >= 4.0 {
        plan.target_duration_sec.clamp(4.0, 120.0)
    } else if spine_dur > 0.5 {
        spine_dur
    } else {
        24.0
    };
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
            let live_project = backend
                .get_project(&project.id)
                .await
                .unwrap_or_else(|_| project.clone());
            let t1_empty = live_project
                .tracks
                .iter()
                .find(|t| t.id == "t1")
                .is_none_or(|t| t.clips.is_empty());
            if !t1_empty {
                return 0;
            }
            let meta = CommandMeta {
                agent_id: "caps-01".to_string(),
                role,
                reason: Some("local captions".to_string()),
            };
            if narrated_transcript_present(&live_project)
                && execute_tool(
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
            if !g1_empty {
                return 0;
            }
            let meta = CommandMeta {
                agent_id: "gfx-01".to_string(),
                role,
                reason: Some("local gfx card".to_string()),
            };
            if !has_visual_media {
                // Zero-asset programmatic video mode: generate full-canvas animated HTML/CSS/SVG/GSAP scenes
                let scenes = build_programmatic_scenes(prompt, plan, brand, target_dur);
                for (idx, (label, start_sec, duration_sec, html)) in scenes.into_iter().enumerate()
                {
                    if cancel.is_cancelled() {
                        break;
                    }
                    if execute_tool(
                        backend.as_ref(),
                        "gfx.place",
                        json!({
                            "project_id": project.id,
                            "clip_id": format!("prog-scene-{idx}-{}", uuid::Uuid::new_v4().simple()),
                            "label": label,
                            "start_sec": start_sec,
                            "duration_sec": duration_sec,
                            "inline_html": html,
                        }),
                        meta.clone(),
                    )
                    .await
                    .is_ok()
                    {
                        ops_counter.fetch_add(1, Ordering::Relaxed);
                        committed += 1;
                    }
                }
            } else if spine_dur > 0.4
                && let Some(asset) = assets.first()
            {
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

fn escape_html_inline(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    if max_chars <= 1 {
        return s.chars().take(max_chars).collect();
    }
    let mut out: String = s.chars().take(max_chars - 1).collect();
    out = out.trim_end().to_string();
    out.push('…');
    out
}

fn extract_topic_title(prompt: &str) -> String {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return "HOW IT WORKS".to_string();
    }
    let joined = trimmed
        .split_whitespace()
        .take(6)
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase();
    truncate_chars(&joined, 56)
}

fn extract_topic_phrase(prompt: &str) -> String {
    let joined = prompt
        .split_whitespace()
        .take(5)
        .collect::<Vec<_>>()
        .join(" ");
    if joined.is_empty() {
        "this story".to_string()
    } else {
        truncate_chars(&joined, 42)
    }
}

fn compute_contiguous_grid(count: usize, total_dur: f64) -> Vec<(f64, f64)> {
    if count == 0 {
        return Vec::new();
    }
    let scene_dur = total_dur / (count as f64);
    let total_rounded = (total_dur * 100.0).round() / 100.0;
    (0..count)
        .map(|idx| {
            let start = ((idx as f64) * scene_dur * 100.0).round() / 100.0;
            let next_start = if idx + 1 == count {
                total_rounded
            } else {
                (((idx + 1) as f64) * scene_dur * 100.0).round() / 100.0
            };
            let dur = ((next_start - start).max(0.1) * 100.0).round() / 100.0;
            (start, dur)
        })
        .collect()
}

fn build_programmatic_scenes(
    prompt: &str,
    plan: &DirectorPlan,
    brand: &BrandPlaybook,
    total_dur: f64,
) -> Vec<(String, f64, f64, String)> {
    let title = extract_topic_title(prompt);
    let topic = extract_topic_phrase(prompt);
    let brand_name = brand.name.clone();

    let specs: Vec<(String, String, String, String)> = if !plan.scenes.is_empty() {
        let count = plan.scenes.len().min(7);
        plan.scenes
            .iter()
            .take(count)
            .enumerate()
            .map(|(idx, s)| {
                let raw_title = if !s.title.trim().is_empty() {
                    truncate_chars(s.title.trim(), 56)
                } else if idx == 0 {
                    title.clone()
                } else {
                    format!("PART {:02}", idx + 1)
                };
                let label = format!("Scene {:02} · {}", idx + 1, truncate_chars(&raw_title, 28));
                let eyebrow = if idx == 0 {
                    "OVERVIEW".to_string()
                } else if idx + 1 == count {
                    brand_name.clone()
                } else {
                    format!("PART {:02}", idx + 1)
                };
                let headline = raw_title.to_uppercase();
                let sub = if !s.visual_brief.trim().is_empty() {
                    truncate_chars(s.visual_brief.trim(), 120)
                } else if !s.narration.trim().is_empty() {
                    truncate_chars(s.narration.trim(), 120)
                } else {
                    format!("Key visual beat for {topic}")
                };
                (label, eyebrow, headline, sub)
            })
            .collect()
    } else {
        vec![
            (
                "Scene 01 · Opening Hook".to_string(),
                "FEATURE".to_string(),
                title,
                "A closer look at what matters most".to_string(),
            ),
            (
                "Scene 02 · The Context".to_string(),
                "ACT 01".to_string(),
                "THE CHALLENGE".to_string(),
                format!("Why {topic} deserves attention right now"),
            ),
            (
                "Scene 03 · Key Insight".to_string(),
                "ACT 02".to_string(),
                "CORE IDEA".to_string(),
                "Breaking down the signal from the noise".to_string(),
            ),
            (
                "Scene 04 · In Motion".to_string(),
                "ACT 03".to_string(),
                "HOW IT WORKS".to_string(),
                "Every element working together in sequence".to_string(),
            ),
            (
                "Scene 05 · The Impact".to_string(),
                "HIGHLIGHT".to_string(),
                "THE PAYOFF".to_string(),
                "Clear results built for lasting momentum".to_string(),
            ),
            (
                "Scene 06 · Closing".to_string(),
                brand_name,
                "TAKE THE NEXT STEP".to_string(),
                format!("Explore {topic} and make it yours"),
            ),
        ]
    };

    let count = specs.len();
    let grid = compute_contiguous_grid(count, total_dur);
    let mut out = Vec::with_capacity(count);
    for (idx, ((label, eyebrow_raw, headline_raw, sub_raw), (start, dur))) in
        specs.into_iter().zip(grid).enumerate()
    {
        let eyebrow = escape_html_inline(&eyebrow_raw);
        let headline = escape_html_inline(&headline_raw);
        let sub = escape_html_inline(&sub_raw);
        let badge = format!("{:02} / {:02}", idx + 1, count);
        let pct = (((idx + 1) as f64 / (count as f64)) * 100.0).round() as u32;
        let html = format!(
            r#"<div class="clip" data-start="0" data-duration="{dur:.2}" style="position:fixed;inset:0;width:100vw;height:100vh;overflow:hidden;background-color:var(--brand-bg,#080404);background-image:radial-gradient(circle at 50% 28%, color-mix(in srgb, var(--brand-accent,#F05A00) 24%, transparent), transparent 70%);font-family:var(--brand-font-display,Sora);color:var(--brand-ink,#F7F5EF);display:flex;flex-direction:column;justify-content:flex-start;align-items:center;padding:8vh 7vw 28vh;">
  <svg viewBox="0 0 1000 1000" preserveAspectRatio="none" style="position:absolute;inset:0;width:100%;height:100%;opacity:0.18;pointer-events:none;">
    <defs><pattern id="g{idx}" width="60" height="60" patternUnits="userSpaceOnUse"><path d="M 60 0 L 0 0 0 60" fill="none" stroke="var(--brand-ink,#F7F5EF)" stroke-width="1"/></pattern></defs>
    <rect width="1000" height="1000" fill="url(#g{idx})"/>
    <circle cx="500" cy="340" r="240" fill="none" stroke="var(--brand-accent,#F05A00)" stroke-width="2" stroke-dasharray="10 12"/>
  </svg>
  <div class="top-bar" style="position:relative;z-index:2;width:100%;display:flex;align-items:center;justify-content:space-between;margin-bottom:6vh;">
    <span style="padding:0.8vh 2.4vw;border-radius:999px;background:var(--brand-surface,#1A0E0E);border:1px solid var(--brand-accent-secondary,#F7B733);font-size:2.4vmin;font-weight:600;letter-spacing:0.16em;color:var(--brand-accent-secondary,#F7B733);">{eyebrow}</span>
    <span style="font-size:2.4vmin;font-weight:600;letter-spacing:0.1em;opacity:0.75;">{badge}</span>
  </div>
  <div class="hero-card" style="position:relative;z-index:2;width:100%;padding:5vh 5vw;border-radius:3.5vmin;background:var(--brand-surface,#1A0E0E);border:1px solid var(--brand-accent,#F05A00);box-shadow:0 24px 60px color-mix(in srgb, var(--brand-bg,#080404) 75%, transparent);text-align:center;">
    <div style="font-size:7.2vmin;font-weight:700;line-height:1.06;letter-spacing:-0.02em;color:var(--brand-ink,#F7F5EF);overflow-wrap:anywhere;">{headline}</div>
    <div style="margin-top:2.2vh;font-size:3.1vmin;font-weight:400;line-height:1.35;color:var(--brand-ink,#F7F5EF);opacity:0.82;font-family:var(--brand-font-body,Sora);">{sub}</div>
    <div style="margin-top:3.4vh;width:100%;height:1.1vh;border-radius:999px;background:var(--brand-bg,#080404);overflow:hidden;">
      <div class="bar-fill" style="width:{pct}%;height:100%;border-radius:999px;background:linear-gradient(90deg,var(--brand-accent,#F05A00),var(--brand-accent-secondary,#F7B733));"></div>
    </div>
  </div>
</div>
<script>
(function(){{
  if (!window.gsap) return;
  window.__timelines = window.__timelines || {{}};
  var tl = window.gsap.timeline({{ paused: true }});
  tl.from(".top-bar", {{ y: -12, opacity: 0.75, duration: 0.35, ease: "power3.out" }}, 0);
  tl.from(".hero-card", {{ y: 18, scale: 0.96, opacity: 0.85, duration: 0.45, ease: "power3.out" }}, 0);
  tl.from(".bar-fill", {{ scaleX: 0.15, transformOrigin: "left center", duration: 0.65, ease: "power2.out" }}, 0.1);
  window.__timelines["main"] = tl;
}})();
</script>"#
        );
        out.push((label, start, dur, html));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{BoxFuture, CapturedFrame, KeyframeResult};
    use crate::lint::lint_gfx_html;
    use crate::swarm::SceneSpec;
    use serde_json::Value;
    use solaris_core_ir::{
        AspectPreset, CaptionBlock, CaptionPreset, CaptionWord, ExportJobStatus, ExportRequest,
        MediaIntelligenceManifest, OpResult, ProjectEngine, TranscriptWord,
    };
    use tokio::sync::RwLock;

    struct DetMockBackend {
        engine: RwLock<ProjectEngine>,
    }

    impl SolarisBackend for DetMockBackend {
        fn list_projects(&self) -> BoxFuture<'_, Result<Vec<Value>, AgentError>> {
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
            _tid: &str,
            _aid: &str,
            _role: AgentRole,
        ) -> BoxFuture<'_, Result<(), AgentError>> {
            Box::pin(async { Ok(()) })
        }
        fn release_track_lock(
            &self,
            _pid: &str,
            _tid: &str,
            _aid: &str,
        ) -> BoxFuture<'_, Result<(), AgentError>> {
            Box::pin(async { Ok(()) })
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
            _ts: f64,
        ) -> BoxFuture<'_, Result<CapturedFrame, AgentError>> {
            Box::pin(async { Err(AgentError::Backend("unused".into())) })
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

    #[test]
    fn programmatic_scenes_have_zero_overlaps_and_valid_gfx_lint() {
        let brand = BrandPlaybook::solaris_default();
        let empty_plan = DirectorPlan::default();
        let long_prompt = "Explain https://example.com/very-long-unbroken-token-that-should-be-capped-cleanly in motion";
        assert!(extract_topic_title(long_prompt).chars().count() <= 56);

        for total_dur in [4.0, 10.0, 16.0, 20.0, 24.0] {
            let scenes = build_programmatic_scenes(long_prompt, &empty_plan, &brand, total_dur);
            assert_eq!(scenes.len(), 6);

            for (idx, pair) in scenes.windows(2).enumerate() {
                let prev_end = pair[0].1 + pair[0].2;
                assert!(
                    prev_end <= pair[1].1 + 1e-9,
                    "scene overlap at total_dur={total_dur} idx={idx}: prev_end={prev_end}, next_start={}",
                    pair[1].1
                );
            }
            let last = scenes.last().unwrap();
            assert!(
                ((last.1 + last.2) - total_dur).abs() <= 0.02,
                "final scene end {} should match total_dur {total_dur}",
                last.1 + last.2
            );

            // Every generated HTML scene passes lint_gfx_html cleanly with zero warnings
            for (label, _, _, html) in &scenes {
                let report = lint_gfx_html(html);
                assert!(
                    report.valid && report.warnings.is_empty(),
                    "scene '{label}' failed lint: errors={:?}, warnings={:?}",
                    report.errors,
                    report.warnings
                );
                assert!(html.contains("if (!window.gsap) return;"));
                assert!(html.contains("var(--brand-bg"));
                assert!(html.contains("var(--brand-surface"));
                assert!(!html.contains("opacity: 0,"));
            }
        }

        // When plan.scenes is populated, fallback derives titles and narration from plan.scenes
        let custom_plan = DirectorPlan {
            target_duration_sec: 12.0,
            scenes: vec![
                SceneSpec {
                    scene_index: 1,
                    title: "Solar Orbit".to_string(),
                    visual_brief: "Orbital rings illuminating the horizon".to_string(),
                    narration: "Sunlight drives every orbit.".to_string(),
                    ..SceneSpec::default()
                },
                SceneSpec {
                    scene_index: 2,
                    title: "Clean Energy".to_string(),
                    visual_brief: "Grid power meters climbing smoothly".to_string(),
                    narration: "Capture more watts every hour.".to_string(),
                    ..SceneSpec::default()
                },
            ],
            ..DirectorPlan::default()
        };
        let planned_scenes = build_programmatic_scenes("Solar energy", &custom_plan, &brand, 12.0);
        assert_eq!(planned_scenes.len(), 2);
        assert!(planned_scenes[0].3.contains("SOLAR ORBIT"));
        assert!(planned_scenes[1].3.contains("CLEAN ENERGY"));
        for (_, _, _, html) in &planned_scenes {
            assert!(lint_gfx_html(html).valid);
        }
    }

    #[tokio::test]
    async fn caps_fallback_skips_non_empty_t1_and_voiceover_only_prefers_spine_duration() {
        let project = Project::new_default("caps_skip_test", AspectPreset::Vertical9x16);
        let stale_snapshot = project.clone();
        let pid = project.id.clone();

        let mut engine = ProjectEngine::new(project);
        // Simulate another worker already having populated T1 after `stale_snapshot` was taken
        engine
            .apply(
                TimelineCommand::PlaceClip {
                    track_id: "t1".to_string(),
                    clip: Clip {
                        id: "existing-cap-1".to_string(),
                        label: "Existing".to_string(),
                        start_sec: 0.0,
                        duration_sec: 2.0,
                        source_in_sec: 0.0,
                        transform: Transform2D::default(),
                        audio: AudioSettings::default(),
                        payload: ClipKind::Caption {
                            caption: CaptionBlock {
                                words: vec![CaptionWord {
                                    text: "Existing".to_string(),
                                    start_sec: 0.0,
                                    end_sec: 2.0,
                                }],
                                preset: CaptionPreset::NeonGlow,
                                font_family: "Sora".to_string(),
                                highlight_color: "#F7B733".to_string(),
                            },
                        },
                    },
                },
                CommandMeta::default(),
            )
            .unwrap();

        let backend: Arc<dyn SolarisBackend> = Arc::new(DetMockBackend {
            engine: RwLock::new(engine),
        });
        let brand = BrandPlaybook::solaris_default();
        let plan = DirectorPlan {
            target_duration_sec: 24.0,
            ..DirectorPlan::default()
        };
        let cancel = CancellationToken::new();
        let ops = Arc::new(AtomicU32::new(0));

        // Passing stale_snapshot (where T1 looked empty) must STILL check live backend and skip!
        let committed = execute_deterministic_specialist(
            &backend,
            &stale_snapshot,
            &brand,
            &[],
            &plan,
            "Explain coffee brewing",
            AgentRole::Caps,
            &cancel,
            &ops,
        )
        .await;
        assert_eq!(committed, 0);
        let live = backend.get_project(&pid).await.unwrap();
        let t1 = live.tracks.iter().find(|t| t.id == "t1").unwrap();
        assert_eq!(t1.clips.len(), 1);
        assert_eq!(t1.clips[0].id, "existing-cap-1");

        // Voiceover-only project (A1 clips, no visual media) prefers spine_dur over plan.target_duration_sec
        let mut vo_proj = Project::new_default("vo_only_test", AspectPreset::Vertical9x16);
        let vo_pid = vo_proj.id.clone();
        vo_proj.media_pool.push(MediaAsset {
            id: "vo-1".to_string(),
            name: "voice.wav".to_string(),
            kind: MediaKind::Audio,
            role_hint: "voiceover".to_string(),
            path: "/tmp/voice.wav".to_string(),
            added_at: vo_proj.created_at,
            intelligence: MediaIntelligenceManifest {
                analyzed: true,
                duration_sec: 9.0,
                ..MediaIntelligenceManifest::default()
            },
        });
        let mut vo_engine = ProjectEngine::new(vo_proj);
        vo_engine
            .apply(
                TimelineCommand::PlaceClip {
                    track_id: "a1".to_string(),
                    clip: Clip {
                        id: "a1-clip".to_string(),
                        label: "VO".to_string(),
                        start_sec: 0.0,
                        duration_sec: 9.0,
                        source_in_sec: 0.0,
                        transform: Transform2D::default(),
                        audio: AudioSettings::default(),
                        payload: ClipKind::Media {
                            media_id: "vo-1".to_string(),
                            ken_burns: false,
                        },
                    },
                },
                CommandMeta::default(),
            )
            .unwrap();
        let vo_snapshot = vo_engine.project().clone();
        let vo_backend: Arc<dyn SolarisBackend> = Arc::new(DetMockBackend {
            engine: RwLock::new(vo_engine),
        });
        let vo_ops = Arc::new(AtomicU32::new(0));
        let gfx_committed = execute_deterministic_specialist(
            &vo_backend,
            &vo_snapshot,
            &brand,
            &[],
            &plan, // target_duration_sec = 24.0, should be overridden by spine_dur = 9.0
            "Voiceover story",
            AgentRole::Gfx,
            &cancel,
            &vo_ops,
        )
        .await;
        assert_eq!(gfx_committed, 6);
        let vo_live = vo_backend.get_project(&vo_pid).await.unwrap();
        let g1 = vo_live.tracks.iter().find(|t| t.id == "g1").unwrap();
        let g1_end = g1
            .clips
            .iter()
            .map(|c| c.start_sec + c.duration_sec)
            .fold(0.0_f64, f64::max);
        assert!(
            (g1_end - 9.0).abs() <= 0.02,
            "expected G1 to span voiceover spine_dur=9.0s, got {g1_end}"
        );
    }
}
