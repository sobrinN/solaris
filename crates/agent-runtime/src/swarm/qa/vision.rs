use std::collections::{HashMap, HashSet};

use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use solaris_core_ir::{AgentRole, LogEntry, LogLevel, Project, QaCheckItem, QaSeverity};
use tokio_util::sync::CancellationToken;

use crate::backend::SolarisBackend;
use crate::openrouter::{
    ChatMessage, ChatRequest, ContentPart, MediaUrlRef, OpenRouterClient, ProviderPreferences,
    ReasoningConfig,
};
use crate::skills::get_skill_resource;
use crate::swarm::llm::{route_models, strip_json_fence};

const MAX_PROGRAMMATIC_QA_FRAMES: usize = 7;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub(super) struct VisionClipFix {
    #[serde(default)]
    pub clip_id: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub defect: String,
    #[serde(default)]
    pub fix_instruction: String,
}

#[derive(Debug, Clone)]
pub(super) struct CapturedClipFrame {
    pub png_base64: String,
    pub local_time_sec: f64,
}

#[derive(Debug, Default)]
pub(super) struct VisionQaInspection {
    pub frames_inspected: usize,
    pub multimodal_summary: Option<String>,
    pub clip_fixes: Vec<VisionClipFix>,
    pub captured_by_clip: HashMap<String, CapturedClipFrame>,
}

#[derive(Debug, Deserialize)]
struct VisionQaResponse {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    issues: Vec<String>,
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    clip_fixes: Vec<VisionClipFix>,
}

pub(super) fn scrub_secrets(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let bytes = raw.as_bytes();
    let lower = raw.to_ascii_lowercase();
    let lower_bytes = lower.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if lower_bytes[i..].starts_with(b"bearer ") {
            out.push_str("Bearer <redacted>");
            i += 7;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            continue;
        }
        if lower_bytes[i..].starts_with(b"sk-or-") {
            out.push_str("<redacted>");
            i += 6;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b'_')
            {
                i += 1;
            }
            continue;
        }
        let ch = raw[i..].chars().next().unwrap_or_default();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn active_span_for_tracks(project: &Project, track_ids: &[&str]) -> Option<(f64, f64)> {
    let mut min_t = f64::INFINITY;
    let mut max_t = 0.0_f64;
    for clip in project
        .tracks
        .iter()
        .filter(|t| !t.muted && (track_ids.is_empty() || track_ids.contains(&t.id.as_str())))
        .flat_map(|t| t.clips.iter())
        .filter(|c| c.start_sec.is_finite() && c.duration_sec.is_finite() && c.duration_sec > 0.0)
    {
        min_t = min_t.min(clip.start_sec.max(0.0));
        max_t = max_t.max(clip.start_sec + clip.duration_sec);
    }
    if min_t.is_finite() && max_t > min_t + 0.05 {
        Some((min_t, max_t))
    } else {
        None
    }
}

pub(super) fn compute_sample_timestamps(span_start: f64, span_end: f64) -> Vec<f64> {
    let span_dur = span_end - span_start;
    if !span_start.is_finite() || !span_end.is_finite() || span_dur <= 0.05 {
        return Vec::new();
    }
    let eps = (span_dur * 0.05).min(0.05);
    let max_t = (span_end - eps).max(span_start);
    let raw_times = [
        (span_start + (span_dur * 0.2).min(0.4)).clamp(span_start, max_t),
        (span_start + span_dur * 0.5).clamp(span_start, max_t),
        (span_start + span_dur * 0.85).clamp(span_start, max_t),
    ];
    let mut sample_times: Vec<f64> = Vec::with_capacity(3);
    for t in raw_times {
        if sample_times.last().is_none_or(|prev| t > *prev + 0.02) {
            sample_times.push(t);
        }
    }
    if sample_times.is_empty() {
        sample_times.push(span_start);
    }
    sample_times
}

fn settled_scene_offset(dur: f64) -> f64 {
    if dur >= 1.8 {
        (dur * 0.68).clamp(1.2, dur - 0.45)
    } else if dur >= 1.0 {
        (dur * 0.65).clamp(0.65, dur - 0.25)
    } else {
        dur * 0.60
    }
}

pub(super) fn compute_project_sample_timestamps(
    project: &Project,
    span_start: f64,
    span_end: f64,
) -> Vec<f64> {
    let has_video_clips = project
        .tracks
        .iter()
        .filter(|t| (t.id == "v1" || t.id == "v2") && !t.muted)
        .any(|t| t.clips.iter().any(|c| c.duration_sec > 0.0));

    if !has_video_clips && let Some(g1) = project.tracks.iter().find(|t| t.id == "g1" && !t.muted) {
        let mut scenes: Vec<(f64, f64)> = g1
            .clips
            .iter()
            .filter(|c| {
                c.start_sec.is_finite() && c.duration_sec.is_finite() && c.duration_sec > 0.05
            })
            .map(|c| (c.start_sec.max(0.0), c.duration_sec))
            .collect();
        scenes.sort_by(|a, b| a.0.total_cmp(&b.0));

        if !scenes.is_empty() {
            // Sample settled mid-scene hero moments across ALL scenes (up to 7) so defects
            // in intermediate scenes (clipped labels, overlapping badges, slide chrome) are never missed.
            let indices: Vec<usize> = if scenes.len() <= MAX_PROGRAMMATIC_QA_FRAMES {
                (0..scenes.len()).collect()
            } else {
                let last = scenes.len() - 1;
                let mut picked = Vec::with_capacity(MAX_PROGRAMMATIC_QA_FRAMES);
                for slot in 0..MAX_PROGRAMMATIC_QA_FRAMES {
                    let idx = (slot * last) / (MAX_PROGRAMMATIC_QA_FRAMES - 1);
                    if picked.last().copied() != Some(idx) {
                        picked.push(idx);
                    }
                }
                picked
            };
            let mut sample_times = Vec::with_capacity(indices.len());
            for idx in indices {
                let (s, dur) = scenes[idx];
                let settled_offset = settled_scene_offset(dur);
                let t = (s + settled_offset).clamp(s, (s + dur - 0.02).max(s));
                if sample_times.last().is_none_or(|prev| t > *prev + 0.02) {
                    sample_times.push(t);
                }
            }
            if !sample_times.is_empty() {
                return sample_times;
            }
        }
    }

    compute_sample_timestamps(span_start, span_end)
}

pub(super) fn compute_clip_sample_timestamps(
    project: &Project,
    clip_ids: &HashSet<String>,
) -> Vec<f64> {
    if clip_ids.is_empty() {
        return Vec::new();
    }
    let Some(g1) = project.tracks.iter().find(|t| t.id == "g1" && !t.muted) else {
        return Vec::new();
    };
    let mut scenes: Vec<(f64, f64)> = g1
        .clips
        .iter()
        .filter(|c| {
            clip_ids.contains(&c.id)
                && c.start_sec.is_finite()
                && c.duration_sec.is_finite()
                && c.duration_sec > 0.05
        })
        .map(|c| (c.start_sec.max(0.0), c.duration_sec))
        .collect();
    scenes.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut sample_times = Vec::with_capacity(scenes.len());
    for (s, dur) in scenes {
        let settled_offset = settled_scene_offset(dur);
        let t = (s + settled_offset).clamp(s, (s + dur - 0.02).max(s));
        if sample_times.last().is_none_or(|prev| t > *prev + 0.02) {
            sample_times.push(t);
        }
    }
    sample_times
}

fn active_g1_clip_at(project: &Project, ts: f64) -> Option<(String, String, f64, f64)> {
    let g1 = project.tracks.iter().find(|t| t.id == "g1" && !t.muted)?;
    g1.clips
        .iter()
        .find(|c| ts >= c.start_sec - 0.01 && ts <= c.start_sec + c.duration_sec + 0.01)
        .map(|c| (c.id.clone(), c.label.clone(), c.start_sec, c.duration_sec))
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn inspect_playback_frames(
    backend: &dyn SolarisBackend,
    openrouter: Option<&OpenRouterClient>,
    vision_model: Option<(&str, &[String])>,
    director_supports_reasoning: bool,
    project: &Project,
    only_clip_ids: Option<&HashSet<String>>,
    cancel: &CancellationToken,
    checks: &mut Vec<QaCheckItem>,
) -> VisionQaInspection {
    if cancel.is_cancelled() {
        return VisionQaInspection::default();
    }

    let sample_times = if let Some(clip_ids) = only_clip_ids {
        compute_clip_sample_timestamps(project, clip_ids)
    } else {
        let Some((span_start, span_end)) = active_span_for_tracks(project, &["v1", "v2"])
            .or_else(|| active_span_for_tracks(project, &["g1"]))
            .or_else(|| active_span_for_tracks(project, &["t1"]))
            .or_else(|| active_span_for_tracks(project, &[]))
        else {
            return VisionQaInspection::default();
        };
        compute_project_sample_timestamps(project, span_start, span_end)
    };
    if sample_times.is_empty() {
        return VisionQaInspection::default();
    }

    let mut captured = Vec::new();
    let mut captured_by_clip: HashMap<String, CapturedClipFrame> = HashMap::new();
    let mut frame_manifest = Vec::new();

    for ts in sample_times {
        if cancel.is_cancelled() {
            break;
        }
        let _ = backend
            .emit_playhead_seek(&project.id, ts, "director-qa", AgentRole::Director)
            .await;
        if let Ok(frame) = backend.capture_preview_frame(&project.id, ts).await {
            let g1_meta = active_g1_clip_at(project, ts);
            let timeline_time_sec = (ts * 100.0).round() / 100.0;
            let (local_time_sec, clip_duration_sec) =
                if let Some((ref cid, _, start_sec, duration_sec)) = g1_meta {
                    let local_t =
                        ((ts - start_sec).clamp(0.0, duration_sec) * 100.0).round() / 100.0;
                    let dur_rounded = (duration_sec * 100.0).round() / 100.0;
                    captured_by_clip
                        .entry(cid.clone())
                        .or_insert_with(|| CapturedClipFrame {
                            png_base64: frame.png_base64.clone(),
                            local_time_sec: local_t,
                        });
                    (Some(local_t), Some(dur_rounded))
                } else {
                    (None, None)
                };
            frame_manifest.push(json!({
                "frame_index": captured.len() + 1,
                "time_sec": timeline_time_sec,
                "local_time_sec": local_time_sec,
                "clip_duration_sec": clip_duration_sec,
                "clip_id": g1_meta.as_ref().map(|(id, _, _, _)| id.as_str()),
                "label": g1_meta.as_ref().map(|(_, lbl, _, _)| lbl.as_str()),
            }));
            captured.push(frame);
        }
    }

    let frames_inspected = captured.len();
    if frames_inspected == 0 {
        checks.push(QaCheckItem {
            id: "visual-frame-skip".to_string(),
            category: "visual".to_string(),
            severity: QaSeverity::Warning,
            message: "Skipped frame capture QA (FFmpeg or playable video frame unavailable at sample timestamps)."
                .to_string(),
            time_sec: None,
            auto_fixed: false,
        });
        return VisionQaInspection::default();
    }

    if only_clip_ids.is_none() {
        checks.push(QaCheckItem {
            id: "visual-frame-capture".to_string(),
            category: "visual".to_string(),
            severity: QaSeverity::Pass,
            message: format!(
                "Captured and inspected {frames_inspected} playback frame(s) across timeline scenes."
            ),
            time_sec: None,
            auto_fixed: false,
        });
    }

    let mut multimodal_summary = None;
    let mut clip_fixes = Vec::new();

    if let (Some(client), Some((model, fallbacks))) = (openrouter, vision_model)
        && client.is_configured()
        && !cancel.is_cancelled()
    {
        let video_clips: usize = project
            .tracks
            .iter()
            .filter(|t| (t.id == "v1" || t.id == "v2") && !t.muted)
            .map(|t| t.clips.len())
            .sum();
        let gfx_clips: usize = project
            .tracks
            .iter()
            .find(|t| t.id == "g1" && !t.muted)
            .map(|t| t.clips.len())
            .unwrap_or(0);
        let caption_clips: usize = project
            .tracks
            .iter()
            .find(|t| t.id == "t1" && !t.muted)
            .map(|t| t.clips.len())
            .unwrap_or(0);

        let is_programmatic_zero_asset = video_clips == 0 && gfx_clips > 0;
        let clip_fixes_schema = "Also populate \"clip_fixes\": [{\"clip_id\": string (exact clip_id from frames), \"severity\": \"issue\" | \"warning\", \"defect\": string, \"fix_instruction\": string}] for EVERY G1 frame that has a visual defect, clipping/overlap, empty placeholder node rings, unsolicited scene/step counter (`01/06`, `ETAPA 02`, `STEP 2`), or useless bottom progress/loading bar so Solaris can auto-repair that clip's HTML.";
        let task_prompt = if is_programmatic_zero_asset {
            if caption_clips > 0 {
                format!(
                    "Review these Solaris programmatic motion-graphics playback frames (each listed with its `clip_id` in `frames`). \
                     IMPORTANT: `programmatic_zero_asset` is TRUE — this project intentionally has 0 imported video_clips and is a pure HTML/CSS/SVG/GSAP motion-graphics video on G1 + captions on T1 (`caption_clips > 0`). \
                     Do NOT report '0 video clips' or 'no camera footage' as an issue, and do NOT penalize intentional vertical clearance in the `64vh..76vh` band reserved for T1 captions as long as the ambient backdrop extends across 100vh. \
                     Evaluate as a harsh supervising motion director: put critical defects (blank/unrendered frame, unreadable/clipped text or labels, badge or dot overlapping text, labels colliding with SVG strokes/shield outlines, severe overlap where captions cover diagrams/headlines, broken layout, heavily blurred ghost cards, or unsolicited `01/06`/`ETAPA 02` counters / useless bottom loading/progress bars) in \"issues\", and minor polish notes (spacing, font sizing, empty hollow node rings without icons, copy redundancy, repetitive identical slide boxes) in \"warnings\". \
                     {clip_fixes_schema} \
                     Reply ONLY with JSON: {{\"summary\": string, \"issues\": [string], \"warnings\": [string], \"clip_fixes\": [...]}}."
                )
            } else {
                format!(
                    "Review these Solaris programmatic motion-graphics playback frames (each listed with its `clip_id` in `frames`). \
                     IMPORTANT: `programmatic_zero_asset` is TRUE and `caption_clips` is 0 — this project intentionally has 0 imported video_clips and NO T1 captions, so G1 scenes should use the full balanced vertical stage (`8vh..84vh` in 9:16) with rich 100vh atmosphere. \
                     Do NOT report '0 video clips' or 'no camera footage' as an issue or defect! \
                     Evaluate as a harsh supervising motion director: put critical defects (blank/unrendered frame, severe bottom-half empty black void, unreadable/clipped text or labels, badge or dot overlapping text, labels colliding with SVG strokes/shield outlines, broken layout, heavily blurred ghost cards, or unsolicited `01/06`/`ETAPA 02` counters / useless bottom loading/progress bars) in \"issues\", and minor polish notes (spacing, font sizing, empty hollow node rings without icons, icon consistency, copy redundancy, repetitive identical slide boxes) in \"warnings\". \
                     {clip_fixes_schema} \
                     Reply ONLY with JSON: {{\"summary\": string, \"issues\": [string], \"warnings\": [string], \"clip_fixes\": [...]}}."
                )
            }
        } else {
            format!(
                "Review these Solaris timeline playback frames for visual completeness, composition, legibility, contrast, and broadcast polish. Put critical defects (blank/pitch-black background, missing scene graphics, unreadable/clipped text, broken framing, unsolicited `01/06`/`ETAPA 02` counters or useless bottom progress bars) in \"issues\" and minor polish notes in \"warnings\". {clip_fixes_schema} Reply ONLY with JSON: {{\"summary\": string, \"issues\": [string], \"warnings\": [string], \"clip_fixes\": [...]}}."
            )
        };
        let qa_rubric = get_skill_resource("skill://director-core/references/critique-and-qa.md")
            .map(|r| format!("\n\n## {}/{}\n{}", r.skill_name, r.relative_path, r.content))
            .unwrap_or_default();
        let base_system_prompt = if is_programmatic_zero_asset {
            "You are Solaris Multimodal QA Director reviewing a programmatic motion-graphics video (G1 HTML/SVG/GSAP + optional T1 captions, intentionally 0 camera clips). Never penalize the absence of camera footage; judge graphic craft, legibility, safe-zone separation, anti-chrome hygiene (no unsolicited 01/06 counters or useless bottom loading bars), and composition honestly using the rubric below. Reply ONLY with valid JSON."
        } else {
            "You are Solaris Multimodal QA Director. Be strict and honest: blank/pitch-black backgrounds, clipped text, overlapping badges, or missing visual graphics are critical issues, not minor warnings. Evaluate every frame using the rubric below. Reply ONLY with valid JSON."
        };
        let system_prompt = format!("{base_system_prompt}{qa_rubric}");

        let mut parts = vec![ContentPart::text(
            json!({
                "task": task_prompt,
                "canvas": project.canvas,
                "active_grade": project.active_grade,
                "frames": frame_manifest,
                "timeline_context": {
                    "video_clips": video_clips,
                    "gfx_clips": gfx_clips,
                    "caption_clips": caption_clips,
                    "programmatic_zero_asset": is_programmatic_zero_asset,
                },
            })
            .to_string(),
        )];
        for frame in &captured {
            parts.push(ContentPart::ImageUrl {
                image_url: MediaUrlRef {
                    url: format!("data:image/png;base64,{}", frame.png_base64),
                },
            });
        }

        let (req_model, req_models) = route_models(model, fallbacks);
        let req = ChatRequest {
            model: req_model,
            models: req_models,
            messages: vec![
                ChatMessage::system_cached(system_prompt),
                ChatMessage::user_parts(parts),
            ],
            session_id: Some(format!("solaris-qa-{}", project.id)),
            provider: Some(ProviderPreferences {
                allow_fallbacks: Some(true),
                ..ProviderPreferences::default()
            }),
            reasoning: director_supports_reasoning.then(|| ReasoningConfig {
                effort: Some("low".to_string()),
                ..ReasoningConfig::default()
            }),
            max_tokens: Some(4096),
            temperature: Some(0.1),
            ..ChatRequest::default()
        };

        match client.chat_stream(&req, Some(cancel), |_| {}).await {
            Ok(resp) => {
                let text = resp
                    .choices
                    .first()
                    .and_then(|c| c.message.content.as_ref())
                    .map(|c| c.as_text())
                    .unwrap_or_default();
                let (summary_opt, parsed_checks, parsed_fixes) = parse_vision_qa_detailed(&text);
                let issue_count = parsed_checks
                    .iter()
                    .filter(|c| c.severity == QaSeverity::Issue)
                    .count();
                let warn_count = parsed_checks
                    .iter()
                    .filter(|c| c.severity == QaSeverity::Warning)
                    .count();
                let log_level = if issue_count > 0 {
                    LogLevel::Warn
                } else if warn_count > 0 {
                    LogLevel::Info
                } else {
                    LogLevel::Debug
                };
                backend.emit_log(LogEntry {
                    id: uuid::Uuid::new_v4().to_string(),
                    timestamp: Utc::now(),
                    level: log_level,
                    source: "qa".to_string(),
                    project_id: Some(project.id.clone()),
                    run_id: None,
                    agent_id: Some("director-qa".to_string()),
                    message: format!(
                        "Vision QA completed ({}, finish={:?}, {}, issues={issue_count}, warnings={warn_count}, clip_fixes={})",
                        resp.model.as_deref().unwrap_or(model),
                        resp.finish_reason(),
                        resp.usage_summary(),
                        parsed_fixes.len()
                    ),
                    details: summary_opt.clone(),
                });
                if summary_opt.is_some() {
                    multimodal_summary = summary_opt;
                }
                clip_fixes = parsed_fixes;
                checks.extend(parsed_checks);
            }
            Err(crate::error::AgentError::Cancelled) => {}
            Err(err) => {
                let safe_err = scrub_secrets(&err.to_string());
                backend.emit_log(LogEntry {
                    id: uuid::Uuid::new_v4().to_string(),
                    timestamp: Utc::now(),
                    level: LogLevel::Warn,
                    source: "qa".to_string(),
                    project_id: Some(project.id.clone()),
                    run_id: None,
                    agent_id: Some("director-qa".to_string()),
                    message: format!("Multimodal vision QA call failed: {safe_err}"),
                    details: Some(format!("model={model}")),
                });
                checks.push(QaCheckItem {
                    id: "vision-qa-error".to_string(),
                    category: "visual".to_string(),
                    severity: QaSeverity::Warning,
                    message: format!("Multimodal vision QA call failed: {safe_err}"),
                    time_sec: None,
                    auto_fixed: false,
                });
            }
        }
    }

    VisionQaInspection {
        frames_inspected,
        multimodal_summary,
        clip_fixes,
        captured_by_clip,
    }
}

#[cfg(test)]
pub(super) fn parse_vision_qa_output(text: &str) -> (Option<String>, Vec<QaCheckItem>) {
    let (summary, checks, _) = parse_vision_qa_detailed(text);
    (summary, checks)
}

pub(super) fn parse_vision_qa_detailed(
    text: &str,
) -> (Option<String>, Vec<QaCheckItem>, Vec<VisionClipFix>) {
    let mut out = Vec::new();
    if text.trim().is_empty() {
        out.push(QaCheckItem {
            id: "vision-qa-empty".to_string(),
            category: "visual".to_string(),
            severity: QaSeverity::Warning,
            message: "Multimodal vision QA returned an empty response.".to_string(),
            time_sec: None,
            auto_fixed: false,
        });
        return (None, out, Vec::new());
    }

    let cleaned = strip_json_fence(text);
    if let Ok(parsed) = serde_json::from_str::<VisionQaResponse>(cleaned) {
        let summary = if parsed.summary.trim().is_empty() {
            None
        } else {
            Some(parsed.summary)
        };
        for (idx, issue) in parsed
            .issues
            .into_iter()
            .filter(|s| !s.trim().is_empty())
            .take(4)
            .enumerate()
        {
            out.push(QaCheckItem {
                id: format!("vision-issue-{idx}"),
                category: "visual".to_string(),
                severity: QaSeverity::Issue,
                message: issue,
                time_sec: None,
                auto_fixed: false,
            });
        }
        for (idx, w) in parsed
            .warnings
            .into_iter()
            .filter(|s| !s.trim().is_empty())
            .take(4)
            .enumerate()
        {
            out.push(QaCheckItem {
                id: format!("vision-warn-{idx}"),
                category: "visual".to_string(),
                severity: QaSeverity::Warning,
                message: w,
                time_sec: None,
                auto_fixed: false,
            });
        }
        let fixes: Vec<VisionClipFix> = parsed
            .clip_fixes
            .into_iter()
            .filter(|f| {
                !f.clip_id.trim().is_empty()
                    && (!f.defect.trim().is_empty() || !f.fix_instruction.trim().is_empty())
            })
            .take(12)
            .collect();
        (summary, out, fixes)
    } else {
        let snippet: String = text.trim().chars().take(180).collect();
        out.push(QaCheckItem {
            id: "vision-qa-parse-error".to_string(),
            category: "visual".to_string(),
            severity: QaSeverity::Warning,
            message: format!("Multimodal vision QA returned non-JSON output: {snippet}"),
            time_sec: None,
            auto_fixed: false,
        });
        (Some(snippet), out, Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use solaris_core_ir::{AspectPreset, AudioSettings, Clip, ClipKind, GfxInstance, Transform2D};

    #[test]
    fn compute_clip_sample_timestamps_samples_only_requested_clips_in_order() {
        let mut proj = Project::new_default("targeted_sample", AspectPreset::Vertical9x16);
        let g1 = proj.tracks.iter_mut().find(|t| t.id == "g1").unwrap();
        for (id, start) in [("c3", 8.0), ("c1", 0.0), ("c2", 4.0)] {
            g1.clips.push(Clip {
                id: id.into(),
                label: id.into(),
                start_sec: start,
                duration_sec: 4.0,
                source_in_sec: 0.0,
                transform: Transform2D::default(),
                audio: AudioSettings::default(),
                payload: ClipKind::Gfx {
                    gfx: GfxInstance {
                        asset_id: None,
                        props: json!({}),
                        inline_html: Some("<div/>".into()),
                    },
                },
            });
        }

        let target: HashSet<String> = ["c3".to_string(), "c1".to_string()].into_iter().collect();
        let times = compute_clip_sample_timestamps(&proj, &target);
        assert_eq!(times.len(), 2);
        assert!((times[0] - 2.72).abs() < 1e-6, "got {}", times[0]);
        assert!((times[1] - 10.72).abs() < 1e-6, "got {}", times[1]);
    }

    #[test]
    fn settled_scene_offset_samples_past_entrance_across_durations() {
        assert!((settled_scene_offset(0.5) - 0.30).abs() < 1e-6);
        assert!((settled_scene_offset(1.2) - 0.78).abs() < 1e-6);
        assert!((settled_scene_offset(3.5) - 2.38).abs() < 1e-6);
        assert!((settled_scene_offset(4.0) - 2.72).abs() < 1e-6);
        assert!((settled_scene_offset(6.0) - 4.08).abs() < 1e-6);
    }
}
