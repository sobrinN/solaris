use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::json;
use solaris_core_ir::{AgentRole, BrandPlaybook, CommandMeta, MediaKind, Project, SavedCodeAsset};
use tokio_util::sync::CancellationToken;

use crate::backend::SolarisBackend;
use crate::error::AgentError;
use crate::openrouter::{
    ChatMessage, ChatRequest, ContentPart, InputAudioPayload, MediaUrlRef, OpenRouterClient,
    ProviderPreferences, truncate_msg,
};
use crate::skills::skills_for_role;
use crate::swarm::{DirectorPlan, is_voice_hint};
use crate::tools::{execute_tool, is_mutating_tool, normalize_tool_name, tools_for_role};

const MAX_LLM_VIDEO_PROXY_BYTES: usize = 4 * 1024 * 1024;

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

fn validate_worker_tool_scope(
    role: AgentRole,
    tool_name: &str,
    args: &serde_json::Value,
    allowed_tracks: &[&str],
) -> Result<(), String> {
    let norm = normalize_tool_name(tool_name.trim());

    // Role-level tool authorization check
    match norm {
        "color.set_grade" if role != AgentRole::Grade && role != AgentRole::Director => {
            return Err(format!(
                "role {role:?} is not permitted to mutate color grade"
            ));
        }
        "gfx.place" if role != AgentRole::Gfx => {
            return Err(format!("role {role:?} is not permitted to place GFX on g1"));
        }
        "captions.generate_from_transcript" | "captions.set_style" if role != AgentRole::Caps => {
            return Err(format!(
                "role {role:?} is not permitted to mutate captions on t1"
            ));
        }
        "audio.enable_ducking" if role != AgentRole::Mix => {
            return Err(format!(
                "role {role:?} is not permitted to mutate a2 ducking"
            ));
        }
        "audio.normalize_lufs" if role != AgentRole::Mix => {
            return Err(format!(
                "role {role:?} is not permitted to normalize speech LUFS"
            ));
        }
        _ => {}
    }

    // Track-level scope enforcement for mutating timeline/audio/color tools
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
    backend: &dyn SolarisBackend,
    openrouter: &OpenRouterClient,
    model: &str,
    fallbacks: &[String],
    video_fallbacks: &[String],
    audio_fallbacks: &[String],
    supports_native_video: bool,
    supports_native_audio: bool,
    project: &Project,
    brand: &BrandPlaybook,
    assets: &[SavedCodeAsset],
    prompt: &str,
    cancel: &CancellationToken,
) -> Result<DirectorPlan, AgentError> {
    if cancel.is_cancelled() {
        return Err(AgentError::Cancelled);
    }

    let skill_text = skills_for_role(AgentRole::Director)
        .into_iter()
        .map(|s| format!("## {}\n{}", s.name, s.content))
        .collect::<Vec<_>>()
        .join("\n\n");

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

    let attached_documents = collect_document_excerpts(backend, project, cancel).await?;

    let asset_names: Vec<String> = assets
        .iter()
        .map(|a| format!("{} ({})", a.id, a.name))
        .collect();

    let sys = format!(
        "You are Solaris Director. Output ONLY valid JSON matching: \
         {{\"hook_summary\": string, \"target_duration_sec\": number, \"spine_notes\": string, \
         \"broll_notes\": string, \"grade_notes\": string, \"captions_notes\": string, \
         \"gfx_notes\": string, \"mix_notes\": string}}.\n\n{skill_text}"
    );
    let user_json = json!({
        "prompt": prompt,
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
    let req = ChatRequest {
        model: req_model,
        models: req_models,
        messages: vec![ChatMessage::system(sys), user_msg],
        session_id: Some(format!("solaris-dir-{}", project.id)),
        provider: Some(ProviderPreferences {
            allow_fallbacks: Some(true),
            require_parameters: require_params,
            ..ProviderPreferences::default()
        }),
        max_tokens: Some(1400),
        temperature: Some(0.2),
        ..ChatRequest::default()
    };

    let resp = openrouter.chat_stream(&req, Some(cancel), |_| {}).await?;
    let text = resp
        .choices
        .first()
        .and_then(|c| c.message.content.as_ref())
        .map(|c| c.as_text())
        .unwrap_or_default();

    let cleaned = strip_json_fence(&text);
    serde_json::from_str::<DirectorPlan>(cleaned).map_err(|e| {
        AgentError::Serialization(format!(
            "Director returned malformed plan JSON: {e} (raw: {})",
            truncate_msg(cleaned)
        ))
    })
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

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_llm_worker_loop(
    backend: &Arc<dyn SolarisBackend>,
    openrouter: &OpenRouterClient,
    model: &str,
    fallbacks: &[String],
    project: &Project,
    brand: &BrandPlaybook,
    assets: &[SavedCodeAsset],
    agent_id: &str,
    role: AgentRole,
    allowed_tracks: &[&str],
    instruction: &str,
    max_turns: usize,
    cancel: &CancellationToken,
    ops_counter: &Arc<AtomicU32>,
) -> Result<u32, AgentError> {
    let skills = skills_for_role(role)
        .into_iter()
        .map(|s| format!("## {}\n{}", s.name, s.content))
        .collect::<Vec<_>>()
        .join("\n\n");

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

    let system_prompt = format!(
        "You are Solaris specialist '{agent_id}' ({role:?}) on project '{}'. \
         You may ONLY mutate tracks {:?}. Call only the provided tools to make clean atomic edits, then reply concisely when finished.\n\n{skills}",
        project.id, allowed_tracks
    );
    let user_prompt = json!({
        "task": instruction,
        "project_id": project.id,
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
    let tools = tools_for_role(role);
    let meta = CommandMeta {
        agent_id: agent_id.to_string(),
        role,
        reason: Some("swarm worker".to_string()),
    };
    let (req_model, req_models) = route_models(model, fallbacks);
    let mut mutating_ops_succeeded = 0u32;
    let mut tool_errors = 0u32;

    for _turn in 0..max_turns {
        if cancel.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        let req = ChatRequest {
            model: req_model.clone(),
            models: req_models.clone(),
            messages: messages.clone(),
            tools: Some(tools.clone()),
            session_id: Some(format!("solaris-{}-{agent_id}", project.id)),
            provider: Some(ProviderPreferences {
                allow_fallbacks: Some(true),
                require_parameters: Some(true),
                ..ProviderPreferences::default()
            }),
            max_tokens: Some(1200),
            temperature: Some(0.2),
            ..ChatRequest::default()
        };

        let resp = openrouter.chat_stream(&req, Some(cancel), |_| {}).await?;
        let Some(choice) = resp.choices.into_iter().next() else {
            break;
        };
        let assistant_msg = choice.message;
        let tool_calls = assistant_msg.tool_calls.clone().unwrap_or_default();
        messages.push(assistant_msg);

        if tool_calls.is_empty() {
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
                    json!({ "ok": false, "error": format!("invalid tool JSON arguments: {parse_err}") })
                        .to_string()
                }
                Ok(args) => {
                    if let Err(scope_err) =
                        validate_worker_tool_scope(role, &tc.function.name, &args, allowed_tracks)
                    {
                        tool_errors += 1;
                        json!({ "ok": false, "error": scope_err }).to_string()
                    } else {
                        let tool_res = tokio::select! {
                            _ = cancel.cancelled() => return Err(AgentError::Cancelled),
                            r = execute_tool(backend.as_ref(), &tc.function.name, args, meta.clone()) => r,
                        };
                        match tool_res {
                            Ok(out) => {
                                if is_mutating_tool(&tc.function.name) {
                                    mutating_ops_succeeded += 1;
                                    ops_counter.fetch_add(1, Ordering::Relaxed);
                                }
                                for b64 in out.all_images_base64() {
                                    captured_images
                                        .push((tc.function.name.clone(), b64.to_string()));
                                }
                                json!({ "ok": true, "summary": out.text, "data": out.structured })
                                    .to_string()
                            }
                            Err(err) => {
                                tool_errors += 1;
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn worker_scope_blocks_cross_track_mutations() {
        assert!(
            validate_worker_tool_scope(
                AgentRole::BRoll,
                "timeline_place_clip",
                &json!({"track_id": "v1"}),
                &["v2"]
            )
            .is_err()
        );
        assert!(
            validate_worker_tool_scope(
                AgentRole::BRoll,
                "timeline_place_clip",
                &json!({"track_id": "v2"}),
                &["v2"]
            )
            .is_ok()
        );
        assert!(
            validate_worker_tool_scope(
                AgentRole::Gfx,
                "color_set_grade",
                &json!({"project_id": "p1"}),
                &["g1"]
            )
            .is_err()
        );
    }
}
