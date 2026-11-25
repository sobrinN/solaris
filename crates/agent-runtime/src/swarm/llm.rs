use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Utc;
use serde_json::json;
use solaris_core_ir::{
    AgentRole, BrandPlaybook, CommandMeta, LogEntry, LogLevel, MediaKind, Project, SavedCodeAsset,
};
use tokio_util::sync::CancellationToken;

use crate::backend::SolarisBackend;
use crate::error::AgentError;
use crate::openrouter::{
    ChatMessage, ChatRequest, ContentPart, InputAudioPayload, MediaUrlRef, MessageContent,
    OpenRouterClient, ProviderPreferences, ReasoningConfig, ToolDefinition, truncate_msg,
};
use crate::planner::CreativePlan;
use crate::skills::{get_skill_resource, skill_references_for_role, skills_for_role};
use crate::swarm::speech::narration_expected;
use crate::swarm::{DirectorPlan, SceneSpec, SwarmRunMode, is_voice_hint};
use crate::tools::{
    execute_tool, is_mutating_tool, normalize_tool_name, tools_for_refinement, tools_for_role,
};

const MAX_LLM_VIDEO_PROXY_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const DIRECTOR_MAX_TOKENS_INITIAL: u32 = 4096;
pub(crate) const DIRECTOR_MAX_TOKENS_RETRY: u32 = 8192;
const WORKER_MAX_TOKENS: u32 = 16384;
const SCENE_WORKER_MAX_TOKENS: u32 = 10240;

#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_swarm_log(
    backend: &dyn SolarisBackend,
    level: LogLevel,
    source: &str,
    project_id: &str,
    run_id: &str,
    agent_id: Option<&str>,
    message: impl Into<String>,
    details: Option<String>,
) {
    backend.emit_log(LogEntry {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: Utc::now(),
        level,
        source: source.to_string(),
        project_id: Some(project_id.to_string()),
        run_id: Some(run_id.to_string()),
        agent_id: agent_id.map(str::to_string),
        message: message.into(),
        details,
    });
}

pub(crate) fn route_models(
    primary: &str,
    fallbacks: &[String],
) -> (Option<String>, Option<Vec<String>>) {
    let mut ordered = vec![primary.to_string()];
    for fb in fallbacks {
        if !ordered.iter().any(|x| x == fb) {
            ordered.push(fb.clone());
        }
    }
    if ordered.len() > 1 {
        (None, Some(ordered))
    } else {
        (Some(primary.to_string()), None)
    }
}

fn brand_context_json(brand: &BrandPlaybook) -> serde_json::Value {
    let ref_notes: Vec<serde_json::Value> = brand
        .reference_media
        .iter()
        .map(|r| {
            json!({
                "id": r.id,
                "name": r.name,
                "kind": r.kind,
                "notes": r.notes,
            })
        })
        .collect();
    json!({
        "id": brand.id,
        "name": brand.name,
        "tokens": brand.tokens,
        "brand_md": brand.brand_md.chars().take(2400).collect::<String>(),
        "pacing_rules": brand.pacing_rules,
        "default_lut": brand.default_lut,
        "reference_media": ref_notes,
    })
}

fn strip_reasoning_details(messages: &mut [ChatMessage]) {
    for msg in messages {
        if msg.role == "assistant" {
            msg.reasoning_details = None;
        }
    }
}

fn push_coaching_user_message(messages: &mut Vec<ChatMessage>, coaching: impl Into<String>) {
    if messages.last().is_some_and(|m| m.role == "user") {
        messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: Some(MessageContent::Text(
                "Understood; retrying with a smaller, valid payload.".to_string(),
            )),
            tool_calls: None,
            tool_call_id: None,
            reasoning_details: None,
        });
    }
    messages.push(ChatMessage::user_text(coaching));
}

fn worker_tools(role: AgentRole, mode: SwarmRunMode) -> Vec<ToolDefinition> {
    match mode {
        SwarmRunMode::Refine => tools_for_refinement(role),
        SwarmRunMode::Auto | SwarmRunMode::Generate => tools_for_role(role),
    }
}

fn validate_worker_tool_scope(
    role: AgentRole,
    tool_name: &str,
    args: &serde_json::Value,
    allowed_tracks: &[&str],
    mode: SwarmRunMode,
) -> Result<(), String> {
    let norm = normalize_tool_name(tool_name.trim());

    // Strict role allowlist enforcement matching `worker_tools(role, mode)`
    let role_allowed = worker_tools(role, mode)
        .into_iter()
        .any(|def| normalize_tool_name(&def.function.name) == norm);
    if !role_allowed {
        return Err(format!(
            "role {role:?} is not permitted to invoke tool '{norm}'"
        ));
    }

    // Track-level scope enforcement for mutating timeline/audio/color tools
    let implicit_track = match norm {
        "gfx.update_clip" | "gfx.place" => Some("g1"),
        "captions.update_clip" | "captions.set_style" => Some("t1"),
        _ => None,
    };
    if let Some(tid) = implicit_track
        && !allowed_tracks.contains(&tid)
    {
        return Err(format!(
            "specialist {role:?} may not mutate track '{tid}' (allowed: {allowed_tracks:?})"
        ));
    }
    if is_mutating_tool(norm)
        && let Some(tid) = args.get("track_id").and_then(|v| v.as_str())
    {
        let permitted = match (role, norm) {
            // Mix owns a2 for placing music, and may tune audio gain/fades on a1/v1 speech
            (AgentRole::Mix, "audio.set_mix") => matches!(tid, "a2" | "a1" | "v1"),
            _ => allowed_tracks.contains(&tid),
        };
        if !permitted {
            return Err(format!(
                "specialist {role:?} may not mutate track '{tid}' (allowed: {allowed_tracks:?})"
            ));
        }
    }

    Ok(())
}

async fn collect_document_excerpts(
    backend: &dyn SolarisBackend,
    project: &Project,
    cancel: &CancellationToken,
) -> Result<Vec<serde_json::Value>, AgentError> {
    let mut attached_documents = Vec::new();
    for doc in project
        .media_pool
        .iter()
        .filter(|m| m.kind == MediaKind::Document && !m.path.is_empty())
        .take(3)
    {
        if cancel.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        let excerpt_res = tokio::select! {
            _ = cancel.cancelled() => return Err(AgentError::Cancelled),
            r = backend.read_document_excerpt(&project.id, &doc.id, 3000) => r,
        };
        if let Ok(Some(excerpt)) = excerpt_res
            && !excerpt.trim().is_empty()
        {
            attached_documents.push(json!({
                "id": doc.id,
                "name": doc.name,
                "excerpt": excerpt,
            }));
        }
    }
    Ok(attached_documents)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn build_openrouter_director_plan(
    backend: &Arc<dyn SolarisBackend>,
    openrouter: &OpenRouterClient,
    model: &str,
    fallbacks: &[String],
    video_fallbacks: &[String],
    audio_fallbacks: &[String],
    supports_native_video: bool,
    supports_native_audio: bool,
    supports_json_mode: bool,
    director_supports_reasoning: bool,
    programmatic_visuals: bool,
    inferred_duration_sec: f64,
    run_id: &str,
    project: &Project,
    brand: &BrandPlaybook,
    assets: &[SavedCodeAsset],
    prompt: &str,
    approved_plan: Option<&CreativePlan>,
    cancel: &CancellationToken,
) -> Result<DirectorPlan, AgentError> {
    if cancel.is_cancelled() {
        return Err(AgentError::Cancelled);
    }

    let mut skill_sections: Vec<String> = skills_for_role(AgentRole::Director)
        .into_iter()
        .map(|s| format!("## {}\n{}", s.name, s.content))
        .collect();
    for r in skill_references_for_role(AgentRole::Director) {
        skill_sections.push(format!("## {}/{}\n{}", r.skill_name, r.relative_path, r.content));
    }
    let skill_text = skill_sections.join("\n\n");

    let media_summary: Vec<serde_json::Value> = project
        .media_pool
        .iter()
        .map(|m| {
            let transcript_preview: Vec<serde_json::Value> = m
                .intelligence
                .transcript
                .iter()
                .take(200)
                .map(|w| json!({ "w": w.word, "s": w.start_sec, "e": w.end_sec }))
                .collect();
            json!({
                "id": m.id,
                "name": m.name,
                "kind": m.kind,
                "role_hint": m.role_hint,
                "duration_sec": m.intelligence.duration_sec,
                "is_log_footage": m.intelligence.is_log_footage,
                "has_audio": m.intelligence.has_audio,
                "silences": m.intelligence.silences,
                "beats": m.intelligence.beats.iter().take(64).copied().collect::<Vec<_>>(),
                "scene_cuts": m.intelligence.scene_cuts.iter().take(48).copied().collect::<Vec<_>>(),
                "transcript_words": m.intelligence.transcript.len(),
                "transcript": transcript_preview,
                "integrated_lufs": m.intelligence.integrated_lufs,
            })
        })
        .collect();

    let attached_documents = collect_document_excerpts(backend.as_ref(), project, cancel).await?;

    let asset_names: Vec<String> = assets
        .iter()
        .map(|a| format!("{} ({})", a.id, a.name))
        .collect();

    let captions_rule = if narration_expected(project) {
        "Captions: transcribed narration is present, so plan captions from its transcript (captions_notes: caption style and timing only)."
    } else {
        "Captions: NO narration with a transcript was detected, so plan no captions and set `captions_notes` to \"No captions: no narration detected\"."
    };
    let mode_guidance = if programmatic_visuals {
        format!(
            "PRODUCTION MODE: PROGRAMMATIC ZERO-ASSET VIDEO (no footage/images imported). \
             Solaris will synthesize the ENTIRE {:.1}s video in PARALLEL using dedicated scene micro-agents on G1. \
             {captions_rule} \
             Do NOT invent fake media_ids or ask for imported footage, and NEVER put internal asset IDs (like `progress-bar-solar`) into visible text specs. \
             In `art_direction`, define ONE unified visual system shared by all scenes (rich full-height layered radial glow + subtle architectural grid backdrop across 100vh so lower regions are never pitch-black, consistent top scene badge format `01 / N`, bold editorial typography, vertical-friendly diagram geometry, crisp brand accent glow). \
             In `scenes`, output 5 to 7 contiguous scene objects covering 0.0s to {:.1}s with ZERO gaps: \
             each object MUST have `scene_index` (1..N), `title`, `start_sec`, `duration_sec`, `visual_brief` (specific diagram/UI/visual focal point contained cleanly inside `5vh..61vh` for 9:16 so `63vh..76vh` stays clear for T1 captions), `motion_cue` (fast headline entrance <= 0.35s + living mid-scene motion, NO fade-to-black before scene end), and `narration` (natural punchy voiceover line that complements the on-screen graphic rather than repeating the headline verbatim). \
             Keep the brief summary in `gfx_notes`. \
             Set `spine_notes`, `broll_notes`, `grade_notes`, and `mix_notes` to brief 1-line notes (\"Programmatic G1/T1 mode\").",
            inferred_duration_sec, inferred_duration_sec
        )
    } else {
        format!(
            "PRODUCTION MODE: FOOTAGE EDIT. Plan cuts, B-roll, color grade, GFX, and audio mix grounded in the supplied media_pool. {captions_rule} Set `scenes` to []."
        )
    };

    let approved_guidance = if approved_plan.is_some() {
        "\nIMPORTANT: The user already reviewed and approved `approved_creative_plan` in the input JSON. Follow its creative direction, approach, style_notes, target_duration_sec, and scene/beat structure faithfully."
    } else {
        ""
    };
    let sys = format!(
        "You are Solaris Director. Output ONLY a single compact, valid raw JSON object (no markdown fences, no commentary) matching:\n\
         {{\"hook_summary\": string, \"target_duration_sec\": number, \"art_direction\": string, \
         \"spine_notes\": string, \"broll_notes\": string, \"grade_notes\": string, \
         \"captions_notes\": string, \"gfx_notes\": string, \"mix_notes\": string, \
         \"scenes\": [{{\"scene_index\": number, \"title\": string, \"start_sec\": number, \"duration_sec\": number, \"visual_brief\": string, \"motion_cue\": string, \"narration\": string}}]}}.\n\
         Keep every field crisp and high-signal so the JSON always terminates cleanly.\n\
         {mode_guidance}{approved_guidance}\n\n{skill_text}"
    );
    let user_json = json!({
        "prompt": prompt,
        "approved_creative_plan": approved_plan,
        "production_mode": if programmatic_visuals { "programmatic_zero_asset" } else { "footage_edit" },
        "suggested_duration_sec": inferred_duration_sec,
        "canvas": project.canvas,
        "brand": brand_context_json(brand),
        "media_pool": media_summary,
        "attached_documents": attached_documents,
        "saved_gfx_assets": asset_names,
    })
    .to_string();

    let mut parts = vec![ContentPart::text(user_json)];
    let mut attached_native_video = false;
    let mut attached_native_audio = false;

    if let Some(primary_video) = project
        .media_pool
        .iter()
        .find(|m| m.kind == MediaKind::Video && !m.path.is_empty())
    {
        if supports_native_video {
            let proxy_res = tokio::select! {
                _ = cancel.cancelled() => return Err(AgentError::Cancelled),
                r = backend.prepare_llm_video_proxy(&project.id, &primary_video.id, MAX_LLM_VIDEO_PROXY_BYTES) => r,
            };
            if let Ok(Some(video_b64)) = proxy_res {
                parts.push(ContentPart::VideoUrl {
                    video_url: MediaUrlRef {
                        url: format!("data:video/mp4;base64,{video_b64}"),
                    },
                });
                attached_native_video = true;
            }
        }

        if !attached_native_video {
            let dur = primary_video.intelligence.duration_sec.max(1.0);
            let ts = vec![
                0.4,
                (dur * 0.45).min(dur - 0.1),
                (dur * 0.85).min(dur - 0.1),
            ];
            let kf_res = tokio::select! {
                _ = cancel.cancelled() => return Err(AgentError::Cancelled),
                r = backend.extract_keyframes(&project.id, &primary_video.id, ts) => r,
            };
            if let Ok(frames) = kf_res {
                for kf in frames.into_iter().take(3) {
                    parts.push(ContentPart::ImageUrl {
                        image_url: MediaUrlRef {
                            url: format!("data:image/png;base64,{}", kf.png_base64),
                        },
                    });
                }
            }
        }
    }

    // Attach up to 4 compact resized image assets so Faceless Explainers & hybrid edits see still visuals
    for img in project
        .media_pool
        .iter()
        .filter(|m| m.kind == MediaKind::Image && !m.path.is_empty())
        .take(4)
    {
        if cancel.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        let img_res = tokio::select! {
            _ = cancel.cancelled() => return Err(AgentError::Cancelled),
            r = backend.read_media_image_base64(&project.id, &img.id) => r,
        };
        if let Ok(Some((mime, b64))) = img_res {
            parts.push(ContentPart::ImageUrl {
                image_url: MediaUrlRef {
                    url: format!("data:{mime};base64,{b64}"),
                },
            });
        }
    }

    // Attach voiceover audio directly when the selected Director model accepts audio input
    if supports_native_audio
        && let Some(voice) = project.media_pool.iter().find(|m| {
            m.kind == MediaKind::Audio
                && !m.path.is_empty()
                && (is_voice_hint(&m.role_hint) || !m.intelligence.transcript.is_empty())
        })
    {
        if cancel.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        let audio_res = tokio::select! {
            _ = cancel.cancelled() => return Err(AgentError::Cancelled),
            r = backend.read_media_audio_base64(&project.id, &voice.id, 30.0) => r,
        };
        if let Ok(Some((format, data))) = audio_res {
            parts.push(ContentPart::InputAudio {
                input_audio: InputAudioPayload { data, format },
            });
            attached_native_audio = true;
        }
    }

    let user_msg = if parts.len() > 1 {
        ChatMessage::user_parts(parts)
    } else if let Some(ContentPart::Text { text, .. }) = parts.into_iter().next() {
        ChatMessage::user_text(text)
    } else {
        ChatMessage::user_text(String::new())
    };

    let active_fallbacks: &[String] = if attached_native_video {
        video_fallbacks
    } else if attached_native_audio {
        audio_fallbacks
    } else {
        fallbacks
    };
    let require_params = (attached_native_video || attached_native_audio).then_some(true);

    let (req_model, req_models) = route_models(model, active_fallbacks);
    let mut messages = vec![ChatMessage::system(sys), user_msg];

    for attempt in 1..=2usize {
        if cancel.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        let max_tokens = if attempt == 1 {
            DIRECTOR_MAX_TOKENS_INITIAL
        } else {
            DIRECTOR_MAX_TOKENS_RETRY
        };
        let use_json_format = attempt == 1 && supports_json_mode;

        emit_swarm_log(
            backend.as_ref(),
            LogLevel::Info,
            "openrouter",
            &project.id,
            run_id,
            Some("director-01"),
            format!(
                "Director planning request (attempt {attempt}/2, model={model}, max_tokens={max_tokens}, json_mode={use_json_format})"
            ),
            None,
        );

        let req = ChatRequest {
            model: req_model.clone(),
            models: req_models.clone(),
            messages: messages.clone(),
            session_id: Some(format!("solaris-dir-{}", project.id)),
            provider: Some(ProviderPreferences {
                allow_fallbacks: Some(true),
                require_parameters: require_params,
                ..ProviderPreferences::default()
            }),
            reasoning: director_supports_reasoning.then(|| ReasoningConfig {
                effort: Some("low".to_string()),
                ..ReasoningConfig::default()
            }),
            max_tokens: Some(max_tokens),
            temperature: Some(if attempt == 1 { 0.2 } else { 0.1 }),
            response_format: use_json_format.then(|| json!({ "type": "json_object" })),
            ..ChatRequest::default()
        };

        let dir_pulse_ms = Arc::new(AtomicU64::new(0));
        let dir_pulse_step = Arc::new(AtomicU32::new(0));
        let dir_backend = backend.clone();
        let dir_pid = project.id.clone();
        let dir_anchors = if programmatic_visuals {
            ["timeline.track.g1", "timeline.track.t1", "effects.library"]
        } else {
            ["media.pool", "timeline.track.v1", "color.inspector"]
        };
        let dir_label = if programmatic_visuals {
            "storyboarding scenes"
        } else {
            "planning edit cuts"
        };
        let resp = match openrouter
            .chat_stream(&req, Some(cancel), |_| {
                let now = now_epoch_ms();
                let prev = dir_pulse_ms.load(Ordering::Relaxed);
                if now.saturating_sub(prev) >= 520
                    && dir_pulse_ms
                        .compare_exchange(prev, now, Ordering::Relaxed, Ordering::Relaxed)
                        .is_ok()
                {
                    let step = dir_pulse_step.fetch_add(1, Ordering::Relaxed) as usize;
                    let anchor = dir_anchors[step % dir_anchors.len()];
                    let b = dir_backend.clone();
                    let pid = dir_pid.clone();
                    tokio::spawn(async move {
                        let _ = b
                            .emit_cursor_pulse(
                                &pid,
                                "director-01",
                                AgentRole::Director,
                                anchor,
                                dir_label,
                            )
                            .await;
                    });
                }
            })
            .await
        {
            Ok(r) => r,
            Err(err) => {
                emit_swarm_log(
                    backend.as_ref(),
                    LogLevel::Error,
                    "openrouter",
                    &project.id,
                    run_id,
                    Some("director-01"),
                    format!("Director OpenRouter call failed on attempt {attempt}: {err}"),
                    Some(format!("model={model}, fallbacks={active_fallbacks:?}")),
                );
                return Err(err);
            }
        };

        let served_model = resp.model.as_deref().unwrap_or(model).to_string();
        let finish_reason = resp.finish_reason().unwrap_or("unknown").to_string();
        let usage = resp.usage_summary();
        let text = resp
            .choices
            .first()
            .and_then(|c| c.message.content.as_ref())
            .map(|c| c.as_text())
            .unwrap_or_default();

        let cleaned = strip_json_fence(&text);
        match parse_director_plan(cleaned, inferred_duration_sec) {
            Ok(plan) => {
                emit_swarm_log(
                    backend.as_ref(),
                    LogLevel::Info,
                    "openrouter",
                    &project.id,
                    run_id,
                    Some("director-01"),
                    format!(
                        "Director plan ready ({served_model}, finish={finish_reason}, {usage}, target={:.1}s)",
                        plan.target_duration_sec
                    ),
                    Some(format!(
                        "hook: {}\ngfx_notes: {}\ncaptions_notes: {}",
                        plan.hook_summary, plan.gfx_notes, plan.captions_notes
                    )),
                );
                return Ok(plan);
            }
            Err(parse_err) => {
                let is_truncated = finish_reason == "length";
                let diagnostic = format!(
                    "model={served_model}\nfinish_reason={finish_reason}\nusage={usage}\nparse_error={parse_err}\nraw_len={} chars\nraw_output:\n{}",
                    cleaned.chars().count(),
                    cleaned.chars().take(6000).collect::<String>()
                );
                if attempt == 1 {
                    emit_swarm_log(
                        backend.as_ref(),
                        LogLevel::Warn,
                        "openrouter",
                        &project.id,
                        run_id,
                        Some("director-01"),
                        format!(
                            "Director returned {} JSON on attempt 1 ({served_model}, finish={finish_reason}, {usage}); retrying with max_tokens={DIRECTOR_MAX_TOKENS_RETRY}",
                            if is_truncated {
                                "truncated"
                            } else {
                                "malformed"
                            }
                        ),
                        Some(diagnostic),
                    );
                    let prev_assistant = if text.trim().is_empty() {
                        "{".to_string()
                    } else {
                        text.chars().take(1200).collect::<String>()
                    };
                    messages.push(ChatMessage {
                        role: "assistant".to_string(),
                        content: Some(MessageContent::Text(prev_assistant)),
                        tool_calls: None,
                        tool_call_id: None,
                        reasoning_details: None,
                    });
                    let retry_prompt = if programmatic_visuals {
                        format!(
                            "Your previous JSON output failed to parse ({parse_err}; finish_reason={finish_reason}). \
                             Output ONLY a single compact, valid, complete raw JSON object (no markdown fences) with keys: \
                             hook_summary, target_duration_sec, art_direction, spine_notes, broll_notes, grade_notes, \
                             captions_notes, gfx_notes, mix_notes, and scenes. \
                             REQUIRED for programmatic mode: include `art_direction` (1-2 crisp sentences) AND a compact `scenes` array \
                             of 5-7 contiguous objects covering 0.0 to {inferred_duration_sec:.1}s with keys \
                             `scene_index`, `title`, `start_sec`, `duration_sec`, `visual_brief`, `motion_cue`, `narration` \
                             (keep every string under 25 words so the JSON terminates cleanly)."
                        )
                    } else {
                        format!(
                            "Your previous JSON output failed to parse ({parse_err}; finish_reason={finish_reason}). \
                             Output ONLY a single compact, valid, complete raw JSON object with keys: \
                             hook_summary, target_duration_sec, art_direction, spine_notes, broll_notes, grade_notes, captions_notes, gfx_notes, mix_notes, scenes ([]). \
                             Keep every string short (1-2 sentences, total JSON under 250 words) and do NOT wrap in markdown code fences."
                        )
                    };
                    messages.push(ChatMessage::user_text(retry_prompt));
                    continue;
                }

                emit_swarm_log(
                    backend.as_ref(),
                    LogLevel::Error,
                    "openrouter",
                    &project.id,
                    run_id,
                    Some("director-01"),
                    format!(
                        "Director returned malformed plan JSON after retry ({served_model}, finish={finish_reason}, {usage}): {parse_err}"
                    ),
                    Some(diagnostic),
                );
                return Err(AgentError::Serialization(format!(
                    "Director returned malformed plan JSON ({served_model}, finish={finish_reason}, {usage}): {parse_err} (raw: {})",
                    truncate_msg(cleaned)
                )));
            }
        }
    }

    Err(AgentError::Serialization(
        "Director failed to produce a valid plan JSON".to_string(),
    ))
}

pub(crate) fn parse_director_plan(
    cleaned: &str,
    fallback_duration_sec: f64,
) -> Result<DirectorPlan, serde_json::Error> {
    let mut plan = serde_json::from_str::<DirectorPlan>(cleaned)?;
    if !plan.target_duration_sec.is_finite() || plan.target_duration_sec < 2.0 {
        plan.target_duration_sec = fallback_duration_sec;
    } else {
        plan.target_duration_sec = plan.target_duration_sec.clamp(3.0, 180.0);
    }
    Ok(plan)
}

pub(crate) fn strip_json_fence(raw: &str) -> &str {
    let trimmed = raw.trim();
    if let Some(fence_start) = trimmed.find("```json") {
        let after = &trimmed[fence_start + "```json".len()..];
        if let Some(fence_end) = after.find("```") {
            return after[..fence_end].trim();
        }
        return after.trim();
    }
    if let Some(fence_start) = trimmed.find("```") {
        let after = &trimmed[fence_start + 3..];
        let body = if let Some(nl) = after.find('\n') {
            &after[nl + 1..]
        } else {
            after
        };
        if let Some(fence_end) = body.find("```") {
            return body[..fence_end].trim();
        }
        return body.trim();
    }
    if let (Some(obj_start), Some(obj_end)) = (trimmed.find('{'), trimmed.rfind('}'))
        && obj_end >= obj_start
    {
        return &trimmed[obj_start..=obj_end];
    }
    trimmed
}

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn default_anchor_for_role(role: AgentRole, allowed_tracks: &[&str]) -> String {
    match role {
        AgentRole::Grade => "color.inspector".to_string(),
        AgentRole::Caps => "timeline.track.t1".to_string(),
        AgentRole::Gfx => "timeline.track.g1".to_string(),
        AgentRole::Mix => "timeline.track.a2".to_string(),
        AgentRole::BRoll => "timeline.track.v2".to_string(),
        AgentRole::Razor => "timeline.track.v1".to_string(),
        _ => allowed_tracks
            .first()
            .map(|t| format!("timeline.track.{t}"))
            .unwrap_or_else(|| "timeline".to_string()),
    }
}

fn anchor_for_tool_call(
    role: AgentRole,
    tool_name: &str,
    args: &serde_json::Value,
    fallback_anchor: &str,
) -> String {
    let norm = normalize_tool_name(tool_name);
    match norm {
        "gfx.place" => {
            if let Some(s) = args.get("start_sec").and_then(|v| v.as_f64())
                && s.is_finite()
                && s >= 0.0
            {
                return format!("timeline.track.g1@{s:.2}");
            }
            "timeline.track.g1".to_string()
        }
        "timeline.place_clip" => {
            let tid = args
                .get("track_id")
                .and_then(|v| v.as_str())
                .unwrap_or("v1");
            if let Some(s) = args.get("start_sec").and_then(|v| v.as_f64())
                && s.is_finite()
                && s >= 0.0
            {
                return format!("timeline.track.{tid}@{s:.2}");
            }
            format!("timeline.track.{tid}")
        }
        "timeline.split_clip" => {
            let tid = args
                .get("track_id")
                .and_then(|v| v.as_str())
                .unwrap_or("v1");
            if let Some(s) = args.get("split_at_timeline_sec").and_then(|v| v.as_f64())
                && s.is_finite()
                && s >= 0.0
            {
                return format!("timeline.track.{tid}@{s:.2}");
            }
            format!("timeline.track.{tid}")
        }
        "captions.place_script" => {
            if let Some(s) = args
                .get("segments")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|seg| seg.get("start_sec"))
                .and_then(|v| v.as_f64())
                && s.is_finite()
                && s >= 0.0
            {
                return format!("timeline.track.t1@{s:.2}");
            }
            "timeline.track.t1".to_string()
        }
        "captions.generate_from_transcript" | "captions.set_style" => {
            "timeline.track.t1".to_string()
        }
        "color.set_grade" | "color.apply_lut" => "color.inspector".to_string(),
        "audio.set_mix" | "audio.normalize_lufs" => "audio.mixer".to_string(),
        "assets.save" | "assets.search" => "effects.library".to_string(),
        _ => {
            let _ = role;
            fallback_anchor.to_string()
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_llm_worker_loop(
    backend: &Arc<dyn SolarisBackend>,
    openrouter: &OpenRouterClient,
    model: &str,
    fallbacks: &[String],
    low_reasoning: bool,
    run_id: &str,
    programmatic_visuals: bool,
    target_duration_sec: f64,
    project: &Project,
    brand: &BrandPlaybook,
    assets: &[SavedCodeAsset],
    agent_id: &str,
    role: AgentRole,
    allowed_tracks: &[&str],
    instruction: &str,
    max_turns: usize,
    mode: SwarmRunMode,
    cancel: &CancellationToken,
    ops_counter: &Arc<AtomicU32>,
) -> Result<u32, AgentError> {
    let mut skill_sections: Vec<String> = skills_for_role(role)
        .into_iter()
        .map(|s| format!("## {}\n{}", s.name, s.content))
        .collect();
    for r in skill_references_for_role(role) {
        skill_sections.push(format!("## {}/{}\n{}", r.skill_name, r.relative_path, r.content));
    }
    let skills = skill_sections.join("\n\n");

    let compact_media: Vec<serde_json::Value> = project
        .media_pool
        .iter()
        .map(|m| {
            json!({
                "id": m.id,
                "name": m.name,
                "kind": m.kind,
                "role_hint": m.role_hint,
                "duration_sec": m.intelligence.duration_sec,
                "is_log_footage": m.intelligence.is_log_footage,
                "has_audio": m.intelligence.has_audio,
                "silences": m.intelligence.silences,
                "beats": m.intelligence.beats.iter().take(48).copied().collect::<Vec<_>>(),
                "scene_cuts": m.intelligence.scene_cuts.iter().take(32).copied().collect::<Vec<_>>(),
                "integrated_lufs": m.intelligence.integrated_lufs,
                "transcript_words": m.intelligence.transcript.len(),
            })
        })
        .collect();

    let compact_tracks: Vec<serde_json::Value> = project
        .tracks
        .iter()
        .filter(|t| !t.clips.is_empty())
        .map(|t| {
            let clips: Vec<serde_json::Value> = t
                .clips
                .iter()
                .map(|c| {
                    json!({
                        "id": c.id,
                        "label": c.label,
                        "start_sec": c.start_sec,
                        "duration_sec": c.duration_sec,
                    })
                })
                .collect();
            json!({ "id": t.id, "code": t.code, "clips": clips })
        })
        .collect();

    let compact_assets: Vec<serde_json::Value> = assets
        .iter()
        .map(|a| {
            json!({
                "id": a.id,
                "name": a.name,
                "default_duration_sec": a.default_duration_sec,
                "default_props": a.default_props,
            })
        })
        .collect();

    let attached_documents = collect_document_excerpts(backend.as_ref(), project, cancel).await?;

    let programmatic_role_brief = if programmatic_visuals {
        match role {
            AgentRole::Gfx => format!(
                "\n\n## PROGRAMMATIC ZERO-ASSET MOTION GRAPHICS MODE\n\
                 No camera footage or images were imported. You are building the ENTIRE {:.1}s visual video programmatically on track `g1`!\n\
                 - Program 6 to 8 distinct, contiguous full-canvas HTML/CSS/SVG/GSAP scenes via `gfx_place` (`inline_html`), covering `0.0s` through `{:.1}s` with ZERO black gaps (each scene ~3.0–4.5s, contiguous `start_sec`).\n\
                 - **CRITICAL PACING & TOKEN RULE**: Place ONLY **1 or 2 scenes per turn**! NEVER batch 4+ full-canvas HTML tool calls into one response (that overflows output tokens and truncates JSON).\n\
                 - **Call `gfx_place` directly** — it ALREADY runs deterministic HTML linting automatically and returns exact errors if anything needs fixing. Do NOT waste tokens calling `gfx_lint_html` first.\n\
                 - Keep each scene compact (~2–4 KB of crisp HTML/CSS/SVG/GSAP) yet visually rich: fill the full canvas (`position:fixed;inset:0;width:100vw;height:100vh;overflow:hidden;`), extend layered gradient/ambient-glow/grid atmosphere across the FULL `100vh` (so the bottom half never looks like a dead pitch-black void), and build bold editorial typography, bespoke inline `<svg viewBox=...>` diagrams, UI cards, and icons.\n\
                 - Use responsive viewport units (`vw`, `vh`, `vmin`, `clamp(...)`, `%`, `<svg viewBox>`) so every scene renders identically in both compact Studio preview and full {}x{} export. In vertical 9:16, stack diagrams vertically (1–2 columns max, NEVER 4 horizontal columns) and keep readable labels `>= 14px`.\n\
                 - `window.gsap` is ALREADY pre-loaded globally offline — NEVER write `<script src=...>` or `<link>` tags; write ONLY inline `<script>...</script>` (`window.__timelines = window.__timelines || {{}}; const tl = gsap.timeline({{paused:true}}); ... window.__timelines['main'] = tl;`) OR CSS `@keyframes`. Complete primary headline entrance by `t <= 0.35s` and keep elements visible (no fade-to-black before exit).\n\
                 - Use brand CSS variables (`--brand-bg`, `--brand-surface`, `--brand-ink`, `--brand-accent`, `--brand-accent-secondary`, `--brand-font-display`, `--brand-font-body`). Never print internal Solaris asset IDs (e.g. `progress-bar-solar`) as visible text.\n\
                 - Non-overlapping vertical bands: in 9:16 portrait, keep ALL foreground G1 content (pill, headline, cards, diagrams) cleanly inside `5vh..61vh` so the `63vh..76vh` band stays 100% clear for `T1` captions (`y: 0.20`) and `78vh..100vh` stays clear for social UI (only ambient backdrop/glow extends full `100vh`). Leave `transform` at `(0,0)` for full-canvas scenes.\n\
                 - Follow strict sandbox rules: root `<div class=\"clip\" data-start=\"0\" data-duration=\"...\">`, ZERO `<script src=...>`, ZERO external URLs/CDNs, ZERO `Math.random`, `Date.now`, `setInterval`, `setTimeout`, `requestAnimationFrame`, `fetch`.\n\
                 - After all scenes cover `0.0s..{:.1}s`, save 1 reusable custom scene template via `assets_save`. Director runs final Vision QA automatically after specialists finish.",
                target_duration_sec,
                target_duration_sec,
                project.canvas.width,
                project.canvas.height,
                target_duration_sec
            ),
            AgentRole::Caps => "\n\n## ZERO-ASSET CAPTIONS MODE\n\
                 No footage or images were imported. Narration with a transcript was detected on the timeline (V1/A1), so call `captions_generate_from_transcript` to caption it. Never invent or script caption text. Reply concisely once done; Director runs full QA automatically after all specialists finish."
                .to_string(),
            _ => String::new(),
        }
    } else {
        String::new()
    };

    let mode_override = if mode == SwarmRunMode::Refine {
        "\n\n## REFINEMENT MODE OVERRIDE\nYou are refining an EXISTING timeline. Only invoke your provided edit-in-place tools (e.g. `gfx_update_clip`, `captions_update_clip`, `timeline_trim_clip`, `timeline_delete_clip`, `timeline_set_transform`, `color_set_grade`, `audio_set_mix`). Ignore any skill instructions that ask to place new clips, regenerate tracks, or lock/unlock tracks."
    } else {
        " Track locking and pre-edit keyframe scouting are handled automatically by the runtime; only call tools present in your provided tool schema."
    };
    let system_prompt = format!(
        "You are Solaris specialist '{agent_id}' ({role:?}) on project '{}'. \
         You may ONLY mutate tracks {:?}. Call only the provided tools to make clean atomic edits, then reply concisely when finished.{mode_override}{programmatic_role_brief}\n\n{skills}",
        project.id, allowed_tracks
    );
    let user_prompt = json!({
        "task": instruction,
        "project_id": project.id,
        "production_mode": if programmatic_visuals { "programmatic_zero_asset" } else { "footage_edit" },
        "target_duration_sec": target_duration_sec,
        "canvas": project.canvas,
        "brand": brand_context_json(brand),
        "media_pool": compact_media,
        "attached_documents": attached_documents,
        "placed_tracks": compact_tracks,
        "saved_gfx_assets": compact_assets,
    })
    .to_string();

    let mut messages = vec![
        ChatMessage::system_cached(system_prompt),
        ChatMessage::user_text_cached(user_prompt),
    ];
    let tools = worker_tools(role, mode);
    let meta = CommandMeta {
        agent_id: agent_id.to_string(),
        role,
        reason: Some("swarm worker".to_string()),
    };
    let mut model_candidates: Vec<String> = vec![model.to_string()];
    for fb in fallbacks {
        if !model_candidates.iter().any(|x| x == fb) {
            model_candidates.push(fb.clone());
        }
    }
    let base_anchor = default_anchor_for_role(role, allowed_tracks);
    let mut active_model_idx: usize = 0;
    let mut mutating_ops_succeeded = 0u32;
    let mut tool_errors = 0u32;
    let mut stream_recoveries = 0u8;
    let mut coverage_nudges = 0u8;

    for turn in 0..max_turns {
        if cancel.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        let current_primary = &model_candidates[active_model_idx];
        let remaining_fallbacks = if active_model_idx + 1 < model_candidates.len() {
            &model_candidates[active_model_idx + 1..]
        } else {
            &[]
        };
        let use_low_reasoning = low_reasoning && active_model_idx == 0;
        let (req_model, req_models) = if use_low_reasoning {
            (Some(current_primary.clone()), None)
        } else {
            route_models(current_primary, remaining_fallbacks)
        };
        let req = ChatRequest {
            model: req_model,
            models: req_models,
            messages: messages.clone(),
            tools: Some(tools.clone()),
            session_id: Some(format!("solaris-{}-{agent_id}", project.id)),
            provider: Some(ProviderPreferences {
                allow_fallbacks: Some(true),
                require_parameters: if use_low_reasoning { None } else { Some(true) },
                ..ProviderPreferences::default()
            }),
            reasoning: use_low_reasoning.then(|| ReasoningConfig {
                effort: Some("low".to_string()),
                ..ReasoningConfig::default()
            }),
            max_tokens: Some(WORKER_MAX_TOKENS),
            temperature: Some(0.2),
            ..ChatRequest::default()
        };

        let last_pulse_ms = Arc::new(AtomicU64::new(0));
        let pulse_backend = backend.clone();
        let pulse_pid = project.id.clone();
        let pulse_agent = agent_id.to_string();
        let pulse_anchor = base_anchor.clone();
        let pulse_label = format!("editing turn {}", turn + 1);

        let resp = match openrouter
            .chat_stream(&req, Some(cancel), |_| {
                let now = now_epoch_ms();
                let prev = last_pulse_ms.load(Ordering::Relaxed);
                if now.saturating_sub(prev) >= 450
                    && last_pulse_ms
                        .compare_exchange(prev, now, Ordering::Relaxed, Ordering::Relaxed)
                        .is_ok()
                {
                    let b = pulse_backend.clone();
                    let pid = pulse_pid.clone();
                    let aid = pulse_agent.clone();
                    let anc = pulse_anchor.clone();
                    let lbl = pulse_label.clone();
                    tokio::spawn(async move {
                        let _ = b.emit_cursor_pulse(&pid, &aid, role, &anc, &lbl).await;
                    });
                }
            })
            .await
        {
            Ok(r) => r,
            Err(err) => {
                let is_recoverable_truncation = matches!(&err, AgentError::StreamError(msg) if {
                    let lower = msg.to_ascii_lowercase();
                    lower.contains("truncated")
                        || lower.contains("invalid json")
                        || lower.contains("missing function name")
                });
                if is_recoverable_truncation && stream_recoveries < 2 && turn + 1 < max_turns {
                    stream_recoveries += 1;
                    tool_errors += 1;
                    let prev_model = current_primary.clone();
                    if active_model_idx + 1 < model_candidates.len() {
                        active_model_idx += 1;
                        strip_reasoning_details(&mut messages);
                    }
                    let next_model = &model_candidates[active_model_idx];
                    emit_swarm_log(
                        backend.as_ref(),
                        LogLevel::Warn,
                        "openrouter",
                        &project.id,
                        run_id,
                        Some(agent_id),
                        format!(
                            "Specialist '{agent_id}' tool call truncated on turn {} ({prev_model}); recovering on {next_model} with smaller per-turn calls",
                            turn + 1
                        ),
                        Some(err.to_string()),
                    );
                    let coaching = if role == AgentRole::Gfx {
                        "Your previous tool call was truncated mid-JSON because the payload was too large. Emit ONLY 1 compact scene per turn by calling `gfx_place` directly (do NOT call `gfx_lint_html` first — `gfx_place` already lints HTML automatically). Keep each scene's `inline_html` concise (~2-4 KB) and continue placing scenes until the full timeline is covered."
                    } else {
                        "Your previous tool call was truncated or had invalid JSON arguments. Emit 1 smaller, valid JSON tool call per turn and continue."
                    };
                    push_coaching_user_message(&mut messages, coaching);
                    continue;
                }
                emit_swarm_log(
                    backend.as_ref(),
                    LogLevel::Error,
                    "openrouter",
                    &project.id,
                    run_id,
                    Some(agent_id),
                    format!(
                        "Specialist '{agent_id}' OpenRouter error on turn {}: {err}",
                        turn + 1
                    ),
                    Some(format!("model={current_primary}")),
                );
                return Err(err);
            }
        };

        let served_model = resp.model.as_deref().unwrap_or(current_primary).to_string();
        let finish_reason = resp.finish_reason().unwrap_or("unknown").to_string();
        let usage = resp.usage_summary();

        let Some(choice) = resp.choices.into_iter().next() else {
            break;
        };
        let assistant_msg = choice.message;
        let tool_calls = assistant_msg.tool_calls.clone().unwrap_or_default();

        emit_swarm_log(
            backend.as_ref(),
            if finish_reason == "length" {
                LogLevel::Warn
            } else {
                LogLevel::Debug
            },
            "openrouter",
            &project.id,
            run_id,
            Some(agent_id),
            format!(
                "{agent_id} turn {} ({served_model}, finish={finish_reason}, {usage}, {} tool call(s))",
                turn + 1,
                tool_calls.len()
            ),
            assistant_msg
                .content
                .as_ref()
                .map(|c| c.as_text())
                .filter(|s| !s.trim().is_empty()),
        );

        messages.push(assistant_msg);

        if tool_calls.is_empty() {
            if programmatic_visuals
                && role == AgentRole::Gfx
                && coverage_nudges < 1
                && turn + 1 < max_turns
                && let Ok(latest) = backend.get_project(&project.id).await
            {
                let (clip_count, covered_sec) =
                    measure_track_coverage(&latest, "g1", target_duration_sec);
                if clip_count == 0 || covered_sec < target_duration_sec * 0.75 {
                    coverage_nudges += 1;
                    emit_swarm_log(
                        backend.as_ref(),
                        LogLevel::Warn,
                        "swarm",
                        &project.id,
                        run_id,
                        Some(agent_id),
                        format!(
                            "{agent_id} paused early with {clip_count} scene(s) covering {covered_sec:.1}s/{target_duration_sec:.1}s; nudging to complete G1"
                        ),
                        None,
                    );
                    messages.push(ChatMessage::user_text(format!(
                        "Track `g1` currently has {clip_count} clip(s) covering only {covered_sec:.1}s of the required {target_duration_sec:.1}s. Call `gfx_place` directly (1-2 scenes per turn, concise ~2-4 KB full-canvas HTML/CSS/SVG/GSAP each) to cover the remaining timeline with ZERO black gaps."
                    )));
                    continue;
                }
            }
            break;
        }

        let mut captured_images: Vec<(String, String)> = Vec::new();

        for tc in tool_calls {
            if cancel.is_cancelled() {
                return Err(AgentError::Cancelled);
            }
            let raw_args = tc.function.arguments.trim();
            let parsed_args: Result<serde_json::Value, _> = if raw_args.is_empty() {
                Ok(json!({}))
            } else {
                serde_json::from_str(raw_args)
            };

            let reply_str = match parsed_args {
                Err(parse_err) => {
                    tool_errors += 1;
                    emit_swarm_log(
                        backend.as_ref(),
                        LogLevel::Warn,
                        "tool",
                        &project.id,
                        run_id,
                        Some(agent_id),
                        format!(
                            "Tool '{}' received malformed JSON args: {parse_err}",
                            tc.function.name
                        ),
                        Some(truncate_msg(raw_args)),
                    );
                    json!({ "ok": false, "error": format!("invalid tool JSON arguments: {parse_err}") })
                        .to_string()
                }
                Ok(args) => {
                    if let Err(scope_err) = validate_worker_tool_scope(
                        role,
                        &tc.function.name,
                        &args,
                        allowed_tracks,
                        mode,
                    ) {
                        tool_errors += 1;
                        emit_swarm_log(
                            backend.as_ref(),
                            LogLevel::Warn,
                            "tool",
                            &project.id,
                            run_id,
                            Some(agent_id),
                            format!(
                                "Blocked out-of-scope tool '{}': {scope_err}",
                                tc.function.name
                            ),
                            None,
                        );
                        json!({ "ok": false, "error": scope_err }).to_string()
                    } else {
                        let tool_anchor =
                            anchor_for_tool_call(role, &tc.function.name, &args, &base_anchor);
                        let tool_res = tokio::select! {
                            _ = cancel.cancelled() => return Err(AgentError::Cancelled),
                            r = execute_tool(backend.as_ref(), &tc.function.name, args, meta.clone()) => r,
                        };
                        match tool_res {
                            Ok(out) => {
                                if is_mutating_tool(&tc.function.name) {
                                    mutating_ops_succeeded += 1;
                                    ops_counter.fetch_add(1, Ordering::Relaxed);
                                    let _ = backend
                                        .emit_cursor_pulse(
                                            &project.id,
                                            agent_id,
                                            role,
                                            &tool_anchor,
                                            &out.text.chars().take(28).collect::<String>(),
                                        )
                                        .await;
                                }
                                emit_swarm_log(
                                    backend.as_ref(),
                                    LogLevel::Info,
                                    "tool",
                                    &project.id,
                                    run_id,
                                    Some(agent_id),
                                    format!("{}: {}", tc.function.name, out.text),
                                    None,
                                );
                                for b64 in out.all_images_base64() {
                                    captured_images
                                        .push((tc.function.name.clone(), b64.to_string()));
                                }
                                json!({ "ok": true, "summary": out.text, "data": out.structured })
                                    .to_string()
                            }
                            Err(err) => {
                                tool_errors += 1;
                                emit_swarm_log(
                                    backend.as_ref(),
                                    LogLevel::Warn,
                                    "tool",
                                    &project.id,
                                    run_id,
                                    Some(agent_id),
                                    format!("Tool '{}' failed: {err}", tc.function.name),
                                    None,
                                );
                                json!({ "ok": false, "error": err.to_string() }).to_string()
                            }
                        }
                    }
                }
            };
            messages.push(ChatMessage::tool_result(tc.id, reply_str));
        }

        if !captured_images.is_empty() {
            let mut img_parts = vec![ContentPart::text(
                "Captured frame(s) from your tool call above for visual inspection:",
            )];
            for (_, b64) in captured_images.into_iter().take(6) {
                img_parts.push(ContentPart::ImageUrl {
                    image_url: MediaUrlRef {
                        url: format!("data:image/png;base64,{b64}"),
                    },
                });
            }
            messages.push(ChatMessage::user_parts(img_parts));
        }
    }

    if mutating_ops_succeeded == 0 && tool_errors > 0 {
        return Err(AgentError::ToolError(format!(
            "specialist '{agent_id}' committed 0 mutating ops ({tool_errors} tool error(s))"
        )));
    }

    Ok(mutating_ops_succeeded)
}

/// Dedicated high-craft micro-agent that authors and places a SINGLE programmatic full-canvas
/// HTML/CSS/SVG/GSAP scene on `g1` in parallel with sibling scene agents.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_programmatic_scene_worker(
    backend: &Arc<dyn SolarisBackend>,
    openrouter: &OpenRouterClient,
    model: &str,
    fallbacks: &[String],
    low_reasoning: bool,
    run_id: &str,
    run_tag: &str,
    project: &Project,
    brand: &BrandPlaybook,
    art_direction: &str,
    prompt: &str,
    scene: &SceneSpec,
    total_scenes: usize,
    save_reusable_asset: bool,
    agent_id: &str,
    mutation_agent_id: &str,
    cancel: &CancellationToken,
    ops_counter: &Arc<AtomicU32>,
) -> Result<u32, AgentError> {
    let scene_anchor = format!("timeline.track.g1@{:.2}", scene.start_sec);
    let _ = backend
        .emit_cursor_pulse(
            &project.id,
            agent_id,
            AgentRole::Gfx,
            &scene_anchor,
            &format!("designing S{}", scene.scene_index),
        )
        .await;

    let clip_id = format!("gfx-s{:02}-{run_tag}", scene.scene_index);
    let save_note = if save_reusable_asset {
        "After placing this scene with `gfx_place`, also save it as a reusable template via `assets_save` (id: `explainer-scene-solar`)."
    } else {
        "Do NOT call `assets_save` (scene 1 handles saving the template)."
    };

    let is_portrait = project.canvas.height > project.canvas.width;
    let layout_zone_rule = if is_portrait {
        "VERTICAL 9:16 SAFE-BAND LAYOUT (CRITICAL — ZERO CAPTION OVERLAP): Keep ALL foreground content (top pill, headline, cards, diagrams, lower badges) strictly between `top: 5vh` and `61vh` (`max-height: 56vh` container starting at `5vh`). NEVER place cards, diagrams, or lower-third bars below `62vh` — `63vh..76vh` is reserved for `T1` kinetic captions (`y=0.20`) and `78vh..100vh` is the bottom social UI safe zone. Stack cards/diagrams vertically (1–2 columns max, NEVER 4 side-by-side horizontal columns in 9:16) and keep all readable body/SVG label text >= 14px (SVG `font-size >= 18` in a `540`-wide `viewBox`)."
    } else {
        "WIDESCREEN/SQUARE SAFE-BAND LAYOUT: Keep ALL foreground content cleanly inside `6vh..72vh` so `75vh..88vh` stays completely clear for `T1` kinetic captions. Keep all readable body/diagram text >= 15px."
    };
    let canonical_badge = format!("{:02} / {:02}", scene.scene_index, total_scenes);

    let craft_refs: String = [
        "skill://captions-and-gfx/references/gfx-contract.md",
        "skill://captions-and-gfx/references/motion-craft.md",
        "skill://captions-and-gfx/references/move-playbook.md",
    ]
    .iter()
    .filter_map(|uri| get_skill_resource(uri))
    .map(|r| format!("## {}/{}\n{}", r.skill_name, r.relative_path, r.content))
    .collect::<Vec<_>>()
    .join("\n\n");

    // Keep the entire system prompt static across sibling scene workers so
    // prefix-based prompt caching hits across all parallel scenes in the run.
    let system_prompt = format!(
        "You are a Solaris motion-design micro-agent authoring a SINGLE programmatic full-canvas scene on track `g1` for the scene assignment provided in the user JSON.\n\
         ## STUDIO CRAFT & SANDBOX CONTRACT\n\
         - Call `gfx_place` DIRECTLY with `inline_html` on Turn 1 (do NOT call `gfx_lint_html` first; `gfx_place` lints automatically and returns exact errors if anything needs fixing).\n\
         - Fill the entire canvas (`position:fixed;inset:0;width:100vw;height:100vh;overflow:hidden;`) with a cohesive, broadcast-grade visual composition: extend rich layered ambient radial glows, subtle architectural grid/mesh lines, and subtle bottom horizon glow across the ENTIRE `100vh` (so the bottom 40% never looks like an empty pitch-black void), with bold editorial typography and bespoke inline `<svg viewBox=...>` diagrams/cards/icons.\n\
         - {layout_zone_rule} Keep `transform` at default `(0,0)`.\n\
         - CONSISTENT SCENE CHROME: Use the EXACT `scene.canonical_badge` from the user JSON (never invent a different denominator like `01/03`). NEVER print internal Solaris asset/tool IDs (such as `progress-bar-solar`, `chapter-title-solar`, `stat-card-solar`, `lower-third-solar`) as visible text.\n\
         - Use responsive viewport units (`vw`, `vh`, `vmin`, `clamp(...)`, `%`, `<svg viewBox>`) so the scene scales crisply across Studio preview and {}x{} export.\n\
         - Use Brand CSS variables (`--brand-bg`, `--brand-surface`, `--brand-ink`, `--brand-accent`, `--brand-accent-secondary`, `--brand-font-display`, `--brand-font-body`) with clean fallbacks matching the brand tokens.\n\
         - GSAP 3 (`window.gsap`) IS ALREADY INJECTED GLOBALLY OFFLINE — NEVER write `<script src=...>` or `<link>` tags! Write ONLY an inline `<script>...</script>` (`window.__timelines = window.__timelines || {{}}; const tl = gsap.timeline({{paused:true}}); ... window.__timelines['main'] = tl;`). Complete the primary headline entrance by `t <= 0.35s` (immediately readable hook!), stagger diagrams/cards cleanly by `0.75s` with living mid-scene motion, and keep all core elements at full opacity through the end of the clip (do NOT fade the scene out to black before exit).\n\
         - Strict sandbox rules: root `<div class=\"clip\" data-start=\"0\" data-duration=\"<scene.duration_sec>\">`, ZERO `<script src=...>`, ZERO external URLs/CDNs, ZERO `Math.random`, `Date.now`, `new Date`, `setInterval`, `setTimeout`, `requestAnimationFrame`, `fetch`, `XMLHttpRequest`, `WebSocket`, `import()`.\n\
         - Keep `inline_html` crisp and focused (~2.5–6 KB) so it renders fast and cleanly.\n\
         - Follow `scene.save_instruction` from the user JSON. Once placed, stop and reply concisely — Director runs Vision QA automatically.\n\n{craft_refs}",
        project.canvas.width,
        project.canvas.height,
    );

    let user_prompt = json!({
        "agent_id": agent_id,
        "user_request": prompt,
        "art_direction": art_direction,
        "scene": {
            "scene_index": scene.scene_index,
            "total_scenes": total_scenes,
            "canonical_badge": canonical_badge,
            "clip_id": clip_id,
            "title": scene.title,
            "start_sec": scene.start_sec,
            "duration_sec": scene.duration_sec,
            "visual_brief": scene.visual_brief,
            "motion_cue": scene.motion_cue,
            "synchronized_narration": scene.narration,
            "save_instruction": save_note,
        },
        "project_id": project.id,
        "canvas": project.canvas,
        "brand": brand_context_json(brand),
    })
    .to_string();

    let mut messages = vec![
        ChatMessage::system_cached(system_prompt),
        ChatMessage::user_text(user_prompt),
    ];
    let allowed_scene_tools: &[&str] = if save_reusable_asset {
        &[
            "gfx.place",
            "gfx.lint_html",
            "brand.get_playbook",
            "assets.save",
        ]
    } else {
        &["gfx.place", "gfx.lint_html", "brand.get_playbook"]
    };
    let tools: Vec<_> = tools_for_role(AgentRole::Gfx)
        .into_iter()
        .filter(|t| allowed_scene_tools.contains(&normalize_tool_name(&t.function.name)))
        .collect();
    let meta = CommandMeta {
        agent_id: mutation_agent_id.to_string(),
        role: AgentRole::Gfx,
        reason: Some(format!(
            "scene {} micro-agent ({agent_id})",
            scene.scene_index
        )),
    };

    let mut model_candidates: Vec<String> = vec![model.to_string()];
    for fb in fallbacks {
        if !model_candidates.iter().any(|x| x == fb) {
            model_candidates.push(fb.clone());
        }
    }
    let mut active_model_idx: usize = 0;
    let mut placed_scene = false;
    let mut saved_asset = !save_reusable_asset;
    let mut mutating_ops_succeeded = 0u32;
    let mut tool_errors = 0u32;
    let mut stream_recoveries = 0u8;
    let max_turns = 4usize;

    for turn in 0..max_turns {
        if cancel.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        let current_primary = &model_candidates[active_model_idx];
        let remaining_fallbacks = if active_model_idx + 1 < model_candidates.len() {
            &model_candidates[active_model_idx + 1..]
        } else {
            &[]
        };
        let use_low_reasoning = low_reasoning && active_model_idx == 0;
        let (req_model, req_models) = if use_low_reasoning {
            (Some(current_primary.clone()), None)
        } else {
            route_models(current_primary, remaining_fallbacks)
        };
        let req = ChatRequest {
            model: req_model,
            models: req_models,
            messages: messages.clone(),
            tools: Some(tools.clone()),
            session_id: Some(format!("solaris-{}-{agent_id}", project.id)),
            provider: Some(ProviderPreferences {
                allow_fallbacks: Some(true),
                require_parameters: if use_low_reasoning { None } else { Some(true) },
                ..ProviderPreferences::default()
            }),
            reasoning: use_low_reasoning.then(|| ReasoningConfig {
                effort: Some("low".to_string()),
                ..ReasoningConfig::default()
            }),
            max_tokens: Some(SCENE_WORKER_MAX_TOKENS),
            temperature: Some(0.25),
            ..ChatRequest::default()
        };

        let last_pulse_ms = Arc::new(AtomicU64::new(0));
        let pulse_backend = backend.clone();
        let pulse_pid = project.id.clone();
        let pulse_agent = agent_id.to_string();
        let pulse_anchor = scene_anchor.clone();
        let pulse_label = format!(
            "S{}: {}",
            scene.scene_index,
            scene.title.chars().take(18).collect::<String>()
        );

        let resp = match openrouter
            .chat_stream(&req, Some(cancel), |_| {
                let now = now_epoch_ms();
                let prev = last_pulse_ms.load(Ordering::Relaxed);
                if now.saturating_sub(prev) >= 450
                    && last_pulse_ms
                        .compare_exchange(prev, now, Ordering::Relaxed, Ordering::Relaxed)
                        .is_ok()
                {
                    let b = pulse_backend.clone();
                    let pid = pulse_pid.clone();
                    let aid = pulse_agent.clone();
                    let anc = pulse_anchor.clone();
                    let lbl = pulse_label.clone();
                    tokio::spawn(async move {
                        let _ = b
                            .emit_cursor_pulse(&pid, &aid, AgentRole::Gfx, &anc, &lbl)
                            .await;
                    });
                }
            })
            .await
        {
            Ok(r) => r,
            Err(err) => {
                let is_recoverable_truncation = matches!(&err, AgentError::StreamError(msg) if {
                    let lower = msg.to_ascii_lowercase();
                    lower.contains("truncated")
                        || lower.contains("invalid json")
                        || lower.contains("missing function name")
                });
                if is_recoverable_truncation && stream_recoveries < 2 && turn + 1 < max_turns {
                    stream_recoveries += 1;
                    tool_errors += 1;
                    if active_model_idx + 1 < model_candidates.len() {
                        active_model_idx += 1;
                        strip_reasoning_details(&mut messages);
                    }
                    emit_swarm_log(
                        backend.as_ref(),
                        LogLevel::Warn,
                        "openrouter",
                        &project.id,
                        run_id,
                        Some(agent_id),
                        format!(
                            "Scene worker '{agent_id}' truncated on turn {}; retrying on {}",
                            turn + 1,
                            model_candidates[active_model_idx]
                        ),
                        Some(err.to_string()),
                    );
                    push_coaching_user_message(
                        &mut messages,
                        "Your previous `gfx_place` JSON was truncated. Emit a cleaner, tighter ~2.5 KB `inline_html` scene directly via `gfx_place`.",
                    );
                    continue;
                }
                emit_swarm_log(
                    backend.as_ref(),
                    LogLevel::Error,
                    "openrouter",
                    &project.id,
                    run_id,
                    Some(agent_id),
                    format!("Scene worker '{agent_id}' OpenRouter error: {err}"),
                    Some(format!("model={current_primary}")),
                );
                return Err(err);
            }
        };

        let served_model = resp.model.as_deref().unwrap_or(current_primary).to_string();
        let finish_reason = resp.finish_reason().unwrap_or("unknown").to_string();
        let usage = resp.usage_summary();
        let Some(choice) = resp.choices.into_iter().next() else {
            break;
        };
        let assistant_msg = choice.message;
        let tool_calls = assistant_msg.tool_calls.clone().unwrap_or_default();

        emit_swarm_log(
            backend.as_ref(),
            if finish_reason == "length" {
                LogLevel::Warn
            } else {
                LogLevel::Debug
            },
            "openrouter",
            &project.id,
            run_id,
            Some(agent_id),
            format!(
                "{agent_id} turn {} ({served_model}, finish={finish_reason}, {usage}, {} tool call(s))",
                turn + 1,
                tool_calls.len()
            ),
            None,
        );

        messages.push(assistant_msg);
        if tool_calls.is_empty() {
            if !placed_scene && turn + 1 < max_turns {
                messages.push(ChatMessage::user_text(format!(
                    "Call `gfx_place` now to place clip `{clip_id}` at `start_sec: {:.2}`, `duration_sec: {:.2}` on `g1`.",
                    scene.start_sec, scene.duration_sec
                )));
                continue;
            }
            break;
        }

        for tc in tool_calls {
            if cancel.is_cancelled() {
                return Err(AgentError::Cancelled);
            }
            let raw_args = tc.function.arguments.trim();
            let parsed_args: Result<serde_json::Value, _> = if raw_args.is_empty() {
                Ok(json!({}))
            } else {
                serde_json::from_str(raw_args)
            };
            let reply_str = match parsed_args {
                Err(parse_err) => {
                    tool_errors += 1;
                    json!({ "ok": false, "error": format!("invalid tool JSON arguments: {parse_err}") })
                        .to_string()
                }
                Ok(mut args) => {
                    // Enforce exact scene window & clip_id on `gfx_place` so parallel scene agents
                    // never collide on duplicate clip IDs or leave timestamp gaps.
                    let norm_name = normalize_tool_name(tc.function.name.trim());
                    if !allowed_scene_tools.contains(&norm_name) {
                        tool_errors += 1;
                        json!({
                            "ok": false,
                            "error": format!("scene micro-agent '{agent_id}' is not permitted to invoke tool '{norm_name}'")
                        })
                        .to_string()
                    } else {
                        if norm_name == "gfx.place"
                            && let Some(obj) = args.as_object_mut()
                        {
                            obj.insert("clip_id".to_string(), json!(clip_id));
                            obj.insert("start_sec".to_string(), json!(scene.start_sec));
                            obj.insert("duration_sec".to_string(), json!(scene.duration_sec));
                            if obj
                                .get("label")
                                .and_then(|v| v.as_str())
                                .is_none_or(|s| s.trim().is_empty())
                            {
                                obj.insert("label".to_string(), json!(scene.title));
                            }
                        }

                        if let Err(scope_err) = validate_worker_tool_scope(
                            AgentRole::Gfx,
                            &tc.function.name,
                            &args,
                            &["g1"],
                            SwarmRunMode::Generate,
                        ) {
                            tool_errors += 1;
                            json!({ "ok": false, "error": scope_err }).to_string()
                        } else {
                            let tool_anchor = anchor_for_tool_call(
                                AgentRole::Gfx,
                                &tc.function.name,
                                &args,
                                &scene_anchor,
                            );
                            let tool_res = tokio::select! {
                                _ = cancel.cancelled() => return Err(AgentError::Cancelled),
                                r = execute_tool(backend.as_ref(), &tc.function.name, args, meta.clone()) => r,
                            };
                            match tool_res {
                                Ok(out) => {
                                    if is_mutating_tool(&tc.function.name) {
                                        mutating_ops_succeeded += 1;
                                        ops_counter.fetch_add(1, Ordering::Relaxed);
                                        if norm_name == "gfx.place" {
                                            placed_scene = true;
                                            let _ = backend
                                                .emit_cursor_pulse(
                                                    &project.id,
                                                    agent_id,
                                                    AgentRole::Gfx,
                                                    &tool_anchor,
                                                    &format!("placed S{}", scene.scene_index),
                                                )
                                                .await;
                                            if scene.scene_index == 1 {
                                                let preview_t = (scene.start_sec
                                                    + scene.duration_sec * 0.35)
                                                    .max(0.4);
                                                let _ = backend
                                                    .emit_playhead_seek(
                                                        &project.id,
                                                        preview_t,
                                                        agent_id,
                                                        AgentRole::Gfx,
                                                    )
                                                    .await;
                                            }
                                        } else if norm_name == "assets.save" {
                                            saved_asset = true;
                                        }
                                    }
                                    emit_swarm_log(
                                        backend.as_ref(),
                                        LogLevel::Info,
                                        "tool",
                                        &project.id,
                                        run_id,
                                        Some(agent_id),
                                        format!("{}: {}", tc.function.name, out.text),
                                        None,
                                    );
                                    json!({ "ok": true, "summary": out.text, "data": out.structured })
                                        .to_string()
                                }
                                Err(err) => {
                                    tool_errors += 1;
                                    emit_swarm_log(
                                        backend.as_ref(),
                                        LogLevel::Warn,
                                        "tool",
                                        &project.id,
                                        run_id,
                                        Some(agent_id),
                                        format!("Tool '{}' failed: {err}", tc.function.name),
                                        None,
                                    );
                                    json!({ "ok": false, "error": err.to_string() }).to_string()
                                }
                            }
                        }
                    }
                }
            };
            messages.push(ChatMessage::tool_result(tc.id, reply_str));
        }

        if placed_scene && saved_asset {
            break;
        }
    }

    if !placed_scene {
        return Err(AgentError::ToolError(format!(
            "scene worker '{agent_id}' did not place scene {} ({tool_errors} tool error(s))",
            scene.scene_index
        )));
    }
    Ok(mutating_ops_succeeded)
}

pub(crate) fn measure_track_coverage(
    project: &Project,
    track_id: &str,
    target_duration_sec: f64,
) -> (usize, f64) {
    let Some(track) = project.tracks.iter().find(|t| t.id == track_id && !t.muted) else {
        return (0, 0.0);
    };
    let count = track.clips.len();
    if count == 0 || !target_duration_sec.is_finite() || target_duration_sec <= 0.0 {
        return (count, 0.0);
    }
    let mut intervals: Vec<(f64, f64)> = track
        .clips
        .iter()
        .filter(|c| c.start_sec.is_finite() && c.duration_sec.is_finite() && c.duration_sec > 0.0)
        .map(|c| {
            (
                c.start_sec.clamp(0.0, target_duration_sec),
                (c.start_sec + c.duration_sec).clamp(0.0, target_duration_sec),
            )
        })
        .filter(|(s, e)| *e > *s)
        .collect();
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut total = 0.0;
    let mut cur_s = 0.0;
    let mut cur_e = 0.0;
    for (s, e) in intervals {
        if s > cur_e {
            total += cur_e - cur_s;
            cur_s = s;
            cur_e = e;
        } else {
            cur_e = cur_e.max(e);
        }
    }
    total += cur_e - cur_s;
    (count, total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use solaris_core_ir::AspectPreset;

    #[test]
    fn strip_json_fence_extracts_json_from_prose_and_code_blocks() {
        let prose_fenced =
            "Here is your plan:\n```json\n{\"hook_summary\":\"Fast hook\"}\n```\nReady!";
        assert_eq!(
            strip_json_fence(prose_fenced),
            r#"{"hook_summary":"Fast hook"}"#
        );

        let prose_raw = "Sure thing! {\"hook_summary\":\"Raw JSON\"} Let me know.";
        assert_eq!(
            strip_json_fence(prose_raw),
            r#"{"hook_summary":"Raw JSON"}"#
        );
    }

    #[test]
    fn worker_scope_blocks_cross_track_mutations_and_out_of_role_tools() {
        let generate = SwarmRunMode::Generate;
        assert!(
            validate_worker_tool_scope(
                AgentRole::BRoll,
                "timeline_place_clip",
                &json!({"track_id": "v1"}),
                &["v2"],
                generate
            )
            .is_err()
        );
        assert!(
            validate_worker_tool_scope(
                AgentRole::BRoll,
                "timeline_place_clip",
                &json!({"track_id": "v2"}),
                &["v2"],
                generate
            )
            .is_ok()
        );
        assert!(
            validate_worker_tool_scope(
                AgentRole::Gfx,
                "color_set_grade",
                &json!({"project_id": "p1"}),
                &["g1"],
                generate
            )
            .is_err()
        );
        for forbidden in [
            "project_undo",
            "project_create",
            "render_export",
            "project_run_qa",
        ] {
            assert!(
                validate_worker_tool_scope(
                    AgentRole::Gfx,
                    forbidden,
                    &json!({"project_id": "p1"}),
                    &["g1"],
                    generate
                )
                .is_err(),
                "expected Gfx to be blocked from {forbidden}"
            );
        }
    }

    #[test]
    fn caption_and_gfx_edit_tools_are_rejected_outside_their_roles() {
        let roles = [
            AgentRole::Razor,
            AgentRole::BRoll,
            AgentRole::Grade,
            AgentRole::Mix,
            AgentRole::Gfx,
            AgentRole::Caps,
        ];
        for role in roles {
            let caption_args = json!({"project_id": "p1", "clip_id": "c1"});
            let caption_ok = validate_worker_tool_scope(
                role,
                "captions_update_clip",
                &caption_args,
                &["t1"],
                SwarmRunMode::Generate,
            )
            .is_ok();
            assert_eq!(
                caption_ok,
                role == AgentRole::Caps,
                "captions_update_clip for {role:?}"
            );

            let gfx_ok = validate_worker_tool_scope(
                role,
                "gfx_update_clip",
                &caption_args,
                &["g1"],
                SwarmRunMode::Generate,
            )
            .is_ok();
            assert_eq!(
                gfx_ok,
                role == AgentRole::Gfx,
                "gfx_update_clip for {role:?}"
            );
        }
    }

    #[test]
    fn gfx_worker_cannot_call_captions_update_clip_in_any_mode() {
        let args = json!({"project_id": "p1", "clip_id": "c1"});
        for mode in [SwarmRunMode::Generate, SwarmRunMode::Refine] {
            assert!(
                validate_worker_tool_scope(
                    AgentRole::Gfx,
                    "captions_update_clip",
                    &args,
                    &["g1", "t1"],
                    mode
                )
                .is_err(),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn refinement_scope_edits_by_id_and_blocks_placement() {
        let refine = SwarmRunMode::Refine;
        let args = json!({"project_id": "p1", "track_id": "g1", "clip_id": "c1"});
        assert!(
            validate_worker_tool_scope(AgentRole::Gfx, "gfx_update_clip", &args, &["g1"], refine)
                .is_ok()
        );
        assert!(
            validate_worker_tool_scope(
                AgentRole::Gfx,
                "timeline_trim_clip",
                &args,
                &["g1"],
                refine
            )
            .is_ok()
        );
        assert!(
            validate_worker_tool_scope(
                AgentRole::Gfx,
                "gfx_place",
                &json!({"project_id": "p1", "clip_id": "n1"}),
                &["g1"],
                refine
            )
            .is_err()
        );
        assert!(
            validate_worker_tool_scope(
                AgentRole::Caps,
                "captions_generate_from_transcript",
                &json!({"project_id": "p1"}),
                &["t1"],
                refine
            )
            .is_err()
        );
        assert!(
            validate_worker_tool_scope(
                AgentRole::Gfx,
                "gfx_place",
                &json!({"project_id": "p1", "clip_id": "n1"}),
                &["g1"],
                SwarmRunMode::Generate
            )
            .is_ok()
        );
    }

    #[test]
    fn message_hygiene_strips_reasoning_and_prevents_consecutive_user_turns() {
        let mut msgs = vec![
            ChatMessage::system("sys"),
            ChatMessage::user_text("u1"),
            ChatMessage {
                role: "assistant".to_string(),
                content: Some(MessageContent::Text("a1".to_string())),
                tool_calls: None,
                tool_call_id: None,
                reasoning_details: Some(json!([{"type": "reasoning.text", "text": "thought"}])),
            },
            ChatMessage::user_text("u2"),
        ];
        strip_reasoning_details(&mut msgs);
        assert!(msgs[2].reasoning_details.is_none());

        push_coaching_user_message(&mut msgs, "retry smaller");
        assert_eq!(msgs.len(), 6);
        assert_eq!(msgs[3].role, "user");
        assert_eq!(msgs[4].role, "assistant");
        assert_eq!(msgs[5].role, "user");
    }

    #[test]
    fn measure_track_coverage_handles_nan_without_panic() {
        let proj = Project::new_default("nan_cov", AspectPreset::Vertical9x16);
        let (count, covered) = measure_track_coverage(&proj, "g1", f64::NAN);
        assert_eq!(count, 0);
        assert_eq!(covered, 0.0);
    }
}
