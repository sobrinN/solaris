use serde::Deserialize;
use serde_json::json;
use solaris_core_ir::{AgentRole, Project, QaCheckItem, QaSeverity};
use tokio_util::sync::CancellationToken;

use crate::backend::SolarisBackend;
use crate::openrouter::{
    ChatMessage, ChatRequest, ContentPart, MediaUrlRef, OpenRouterClient, ProviderPreferences,
};
use crate::swarm::llm::{route_models, strip_json_fence};

#[derive(Debug, Deserialize)]
struct VisionQaResponse {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    warnings: Vec<String>,
}

pub(super) async fn inspect_playback_frames(
    backend: &dyn SolarisBackend,
    openrouter: Option<&OpenRouterClient>,
    vision_model: Option<(&str, &[String])>,
    project: &Project,
    cancel: &CancellationToken,
    checks: &mut Vec<QaCheckItem>,
) -> (usize, Option<String>) {
    let mut visual_min = f64::INFINITY;
    let mut visual_max = 0.0_f64;
    for clip in project
        .tracks
        .iter()
        .filter(|t| (t.id == "v1" || t.id == "v2") && !t.muted)
        .flat_map(|t| t.clips.iter())
    {
        visual_min = visual_min.min(clip.start_sec.max(0.0));
        visual_max = visual_max.max(clip.start_sec + clip.duration_sec);
    }

    let (span_start, span_end) =
        if visual_max > 0.1 && visual_min.is_finite() && visual_max > visual_min {
            (visual_min, visual_max)
        } else {
            (0.0, project.duration_sec())
        };
    let span_dur = span_end - span_start;
    if span_dur <= 0.1 || cancel.is_cancelled() {
        return (0, None);
    }

    let sample_times = vec![
        span_start + (span_dur * 0.2).min(0.4),
        span_start + span_dur * 0.5,
        span_start + (span_dur * 0.85).min((span_dur - 0.1).max(0.0)),
    ];

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
        let mut parts = vec![ContentPart::text(
            json!({
                "task": "Review these Solaris timeline playback frames (hook, mid, climax) for framing, contrast, and visual polish. Reply ONLY with JSON: {\"summary\": string, \"warnings\": [string]}.",
                "canvas": project.canvas,
                "active_grade": project.active_grade,
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
                    "You are Solaris Multimodal QA Director. Reply ONLY with valid JSON.",
                ),
                ChatMessage::user_parts(parts),
            ],
            session_id: Some(format!("solaris-qa-{}", project.id)),
            provider: Some(ProviderPreferences {
                allow_fallbacks: Some(true),
                ..ProviderPreferences::default()
            }),
            max_tokens: Some(400),
            temperature: Some(0.1),
            ..ChatRequest::default()
        };

        match client.chat_stream(&req, Some(cancel), |_| {}).await {
            Ok(resp) => {
                if let Some(text) = resp
                    .choices
                    .first()
                    .and_then(|c| c.message.content.as_ref())
                    .map(|c| c.as_text())
                {
                    let cleaned = strip_json_fence(&text);
                    if let Ok(parsed) = serde_json::from_str::<VisionQaResponse>(cleaned) {
                        if !parsed.summary.trim().is_empty() {
                            multimodal_summary = Some(parsed.summary);
                        }
                        for (idx, w) in parsed.warnings.into_iter().take(3).enumerate() {
                            if !w.trim().is_empty() {
                                checks.push(QaCheckItem {
                                    id: format!("vision-warn-{idx}"),
                                    category: "visual".to_string(),
                                    severity: QaSeverity::Warning,
                                    message: w,
                                    time_sec: None,
                                    auto_fixed: false,
                                });
                            }
                        }
                    } else if !text.trim().is_empty() {
                        multimodal_summary = Some(text.chars().take(180).collect());
                    }
                }
            }
            Err(crate::error::AgentError::Cancelled) => {}
            Err(err) => {
                checks.push(QaCheckItem {
                    id: "vision-qa-error".to_string(),
                    category: "visual".to_string(),
                    severity: QaSeverity::Warning,
                    message: format!("Multimodal vision QA call failed: {err}"),
                    time_sec: None,
                    auto_fixed: false,
                });
            }
        }
    }

    (frames_inspected, multimodal_summary)
}
