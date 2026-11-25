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
use crate::swarm::llm::{route_models, strip_json_fence};

#[derive(Debug, Deserialize)]
struct VisionQaResponse {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    issues: Vec<String>,
    #[serde(default)]
    warnings: Vec<String>,
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

#[allow(clippy::too_many_arguments)]
pub(super) async fn inspect_playback_frames(
    backend: &dyn SolarisBackend,
    openrouter: Option<&OpenRouterClient>,
    vision_model: Option<(&str, &[String])>,
    director_supports_reasoning: bool,
    project: &Project,
    cancel: &CancellationToken,
    checks: &mut Vec<QaCheckItem>,
) -> (usize, Option<String>) {
    let Some((span_start, span_end)) = active_span_for_tracks(project, &["v1", "v2"])
        .or_else(|| active_span_for_tracks(project, &["g1"]))
        .or_else(|| active_span_for_tracks(project, &["t1"]))
        .or_else(|| active_span_for_tracks(project, &[]))
    else {
        return (0, None);
    };
    if cancel.is_cancelled() {
        return (0, None);
    }

    let sample_times = compute_sample_timestamps(span_start, span_end);
    if sample_times.is_empty() {
        return (0, None);
    }

    let mut captured = Vec::new();
    for ts in sample_times {
        if cancel.is_cancelled() {
            break;
        }
        let _ = backend
            .emit_playhead_seek(&project.id, ts, "director-qa", AgentRole::Director)
            .await;
        if let Ok(frame) = backend.capture_preview_frame(&project.id, ts).await {
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
        return (0, None);
    }

    checks.push(QaCheckItem {
        id: "visual-frame-capture".to_string(),
        category: "visual".to_string(),
        severity: QaSeverity::Pass,
        message: format!(
            "Captured and inspected {frames_inspected} playback frame(s) (hook/mid/climax)."
        ),
        time_sec: None,
        auto_fixed: false,
    });

    let mut multimodal_summary = None;
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

        let mut parts = vec![ContentPart::text(
            json!({
                "task": "Review these Solaris timeline playback frames (hook, mid, climax) for visual completeness, composition, legibility, contrast, and broadcast polish. Put critical defects (blank/pitch-black background, missing scene graphics, unreadable/clipped text, broken framing) in \"issues\" and minor polish notes in \"warnings\". Reply ONLY with JSON: {\"summary\": string, \"issues\": [string], \"warnings\": [string]}.",
                "canvas": project.canvas,
                "active_grade": project.active_grade,
                "timeline_context": {
                    "video_clips": video_clips,
                    "gfx_clips": gfx_clips,
                    "caption_clips": caption_clips,
                    "programmatic_zero_asset": video_clips == 0,
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
                ChatMessage::system(
                    "You are Solaris Multimodal QA Director. Be strict and honest: blank/pitch-black backgrounds or missing visual graphics are critical issues, not minor warnings. Reply ONLY with valid JSON.",
                ),
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
            max_tokens: Some(2560),
            temperature: Some(0.1),
            ..ChatRequest::default()
        };

        match client.chat_stream(&req, Some(cancel), |_| {}).await {
            Ok(resp) => {
                backend.emit_log(LogEntry {
                    id: uuid::Uuid::new_v4().to_string(),
                    timestamp: Utc::now(),
                    level: LogLevel::Debug,
                    source: "qa".to_string(),
                    project_id: Some(project.id.clone()),
                    run_id: None,
                    agent_id: Some("director-qa".to_string()),
                    message: format!(
                        "Vision QA completed ({}, finish={:?}, {})",
                        resp.model.as_deref().unwrap_or(model),
                        resp.finish_reason(),
                        resp.usage_summary()
                    ),
                    details: None,
                });
                let text = resp
                    .choices
                    .first()
                    .and_then(|c| c.message.content.as_ref())
                    .map(|c| c.as_text())
                    .unwrap_or_default();
                let (summary_opt, parsed_checks) = parse_vision_qa_output(&text);
                if summary_opt.is_some() {
                    multimodal_summary = summary_opt;
                }
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

    (frames_inspected, multimodal_summary)
}

pub(super) fn parse_vision_qa_output(text: &str) -> (Option<String>, Vec<QaCheckItem>) {
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
        return (None, out);
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
        (summary, out)
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
        (Some(snippet), out)
    }
}
