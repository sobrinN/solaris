use serde::{Deserialize, Serialize};
use serde_json::json;
use solaris_core_ir::{AspectPreset, BrandPlaybook, MediaAsset, MediaKind, SavedCodeAsset};
use tokio_util::sync::CancellationToken;

use crate::advisor::{ModelAdvisor, ModelRoleKind};
use crate::error::AgentError;
use crate::openrouter::{
    ChatMessage, ChatRequest, OpenRouterClient, ProviderPreferences, ReasoningConfig, truncate_msg,
};
use crate::skills::{get_skill_resource, list_skills};
use crate::swarm::{infer_target_duration_from_prompt, route_models, strip_json_fence};

const PLANNER_MAX_TOKENS_INITIAL: u32 = 3072;
const PLANNER_MAX_TOKENS_RETRY: u32 = 4096;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CreativePlanBeat {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub time_range: String,
    #[serde(default)]
    pub visual_direction: String,
    #[serde(default)]
    pub audio_and_captions: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CreativePlan {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub approach: String,
    #[serde(default)]
    pub creative_direction: String,
    #[serde(default)]
    pub target_duration_sec: f64,
    #[serde(default)]
    pub production_mode: String,
    #[serde(default)]
    pub skills_consulted: Vec<String>,
    #[serde(default)]
    pub beats: Vec<CreativePlanBeat>,
    #[serde(default)]
    pub style_notes: Vec<String>,
    #[serde(default)]
    pub model_used: String,
    #[serde(default)]
    pub used_local_fallback: bool,
}

pub struct CreativePlanInput<'a> {
    pub prompt: &'a str,
    pub aspect_preset: AspectPreset,
    pub brand: &'a BrandPlaybook,
    pub media_pool: &'a [MediaAsset],
    pub saved_assets: &'a [SavedCodeAsset],
    pub model_override: Option<&'a str>,
    pub refinement_note: Option<&'a str>,
    pub previous_plan: Option<&'a CreativePlan>,
}

fn build_skills_brief() -> (Vec<String>, String) {
    let skills = list_skills();
    let names: Vec<String> = skills.iter().map(|s| s.name.clone()).collect();
    let mut sections: Vec<String> = skills
        .into_iter()
        .map(|s| format!("## Skill: {}\n{}", s.name, s.content))
        .collect();
    if let Some(pacing_ref) =
        get_skill_resource("skill://director-core/references/direction-and-pacing.md")
    {
        sections.push(format!(
            "## Reference: direction-and-pacing\n{}",
            pacing_ref.content
        ));
    }
    (names, sections.join("\n\n"))
}

fn media_summary_json(media_pool: &[MediaAsset]) -> Vec<serde_json::Value> {
    media_pool
        .iter()
        .take(24)
        .map(|m| {
            let transcript_preview: String = m
                .intelligence
                .transcript
                .iter()
                .take(40)
                .map(|w| w.word.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            json!({
                "id": m.id,
                "name": m.name,
                "kind": m.kind,
                "role_hint": m.role_hint,
                "duration_sec": m.intelligence.duration_sec,
                "has_audio": m.intelligence.has_audio,
                "is_log_footage": m.intelligence.is_log_footage,
                "silence_count": m.intelligence.silences.len(),
                "beat_count": m.intelligence.beats.len(),
                "transcript_preview": transcript_preview,
            })
        })
        .collect()
}

fn aspect_label(aspect: AspectPreset) -> &'static str {
    match aspect {
        AspectPreset::Vertical9x16 => "9:16 vertical (1080x1920)",
        AspectPreset::Landscape16x9 => "16:9 widescreen (1920x1080)",
        AspectPreset::Square1x1 => "1:1 square (1080x1080)",
    }
}

fn has_visual_media(media_pool: &[MediaAsset]) -> bool {
    media_pool
        .iter()
        .any(|m| matches!(m.kind, MediaKind::Video | MediaKind::Image))
}

fn normalize_creative_plan(
    mut plan: CreativePlan,
    fallback_duration_sec: f64,
    programmatic: bool,
    available_skills: &[String],
    model_used: &str,
    used_local_fallback: bool,
) -> CreativePlan {
    if plan.title.trim().is_empty() {
        plan.title = "Custom Solaris Direction".to_string();
    }
    if plan.approach.trim().is_empty() {
        plan.approach = if programmatic {
            "Programmatic Motion Film".to_string()
        } else {
            "Custom Editorial Cut".to_string()
        };
    }
    if !plan.target_duration_sec.is_finite() || plan.target_duration_sec < 3.0 {
        plan.target_duration_sec = fallback_duration_sec.clamp(4.0, 180.0);
    } else {
        plan.target_duration_sec = plan.target_duration_sec.clamp(4.0, 180.0);
    }
    if plan.production_mode.trim().is_empty() {
        plan.production_mode = if programmatic {
            "programmatic".to_string()
        } else {
            "footage_edit".to_string()
        };
    }
    if plan.skills_consulted.is_empty() {
        plan.skills_consulted = available_skills.to_vec();
    } else {
        plan.skills_consulted.retain(|s| !s.trim().is_empty());
        plan.skills_consulted.truncate(8);
    }
    plan.beats
        .retain(|b| !b.title.trim().is_empty() || !b.visual_direction.trim().is_empty());
    plan.beats.truncate(10);
    plan.style_notes.retain(|n| !n.trim().is_empty());
    plan.style_notes.truncate(8);
    plan.model_used = model_used.to_string();
    plan.used_local_fallback = used_local_fallback;
    plan
}

pub fn build_local_creative_plan(input: &CreativePlanInput<'_>) -> CreativePlan {
    let programmatic = !has_visual_media(input.media_pool);
    let media_total_sec: f64 = input
        .media_pool
        .iter()
        .filter(|m| matches!(m.kind, MediaKind::Video | MediaKind::Audio))
        .map(|m| m.intelligence.duration_sec.max(0.0))
        .sum();
    let duration = infer_target_duration_from_prompt(input.prompt, media_total_sec);
    let topic = input
        .prompt
        .trim()
        .chars()
        .take(56)
        .collect::<String>()
        .trim()
        .to_string();
    let title = if topic.is_empty() {
        "Custom Solaris Edit".to_string()
    } else {
        topic
    };

    let (all_skills, _) = build_skills_brief();
    let step = (duration / 4.0).max(1.0);
    let fmt_range = |i: usize| -> String {
        let start = (i as f64) * step;
        let end = if i == 3 {
            duration
        } else {
            (i + 1) as f64 * step
        };
        format!("{start:.1}s-{end:.1}s")
    };

    let (approach, creative_direction, beats, style_notes) = if programmatic {
        let dir = format!(
            "Programmatic {} motion piece built directly around your prompt using brand '{}' tokens ({}, {}) and responsive HTML/SVG/GSAP scenes (T1 captions are added only if narration audio is present).",
            aspect_label(input.aspect_preset),
            input.brand.name,
            input.brand.tokens.accent,
            input.brand.tokens.accent_secondary
        );
        let beats = vec![
            CreativePlanBeat {
                title: "Opening Hook".to_string(),
                time_range: fmt_range(0),
                visual_direction: format!(
                    "Full-canvas bold typographic reveal framing \"{title}\" with high-contrast focal geometry across the active stage."
                ),
                audio_and_captions: "Visual-only beat (T1 captions generated only if transcribed narration audio is attached).".to_string(),
            },
            CreativePlanBeat {
                title: "Core Idea & Visual Breakdown".to_string(),
                time_range: fmt_range(1),
                visual_direction: "Custom SVG diagram / UI card sequence illustrating the main mechanism with purposeful GSAP choreography.".to_string(),
                audio_and_captions: "Visual-only beat; motion and kinetic typography carry the explanation without narration.".to_string(),
            },
            CreativePlanBeat {
                title: "Proof & Contrast Moment".to_string(),
                time_range: fmt_range(2),
                visual_direction: "Metric callout, before/after split, or animated comparison highlighting the payoff.".to_string(),
                audio_and_captions: "Visual-only beat; high-contrast on-screen callouts emphasize the key takeaway.".to_string(),
            },
            CreativePlanBeat {
                title: "Closing Payoff".to_string(),
                time_range: fmt_range(3),
                visual_direction: "Clean brand-anchored end frame resolving the motion language with a crisp closing card.".to_string(),
                audio_and_captions: "Visual-only closing frame (T1 captions generated only when narration audio exists).".to_string(),
            },
        ];
        let notes = vec![
            format!("Canvas: {}", aspect_label(input.aspect_preset)),
            format!("Brand: {} ({})", input.brand.name, input.brand.tokens.font_display),
            "Programmatic G1 full-canvas GSAP scenes (no raw footage required; T1 captions added only when transcribed narration audio exists)".to_string(),
            "Connect an OpenRouter API key in AI & Keys to generate bespoke LLM storyboards".to_string(),
        ];
        (
            "Programmatic Motion Piece (Local Draft)".to_string(),
            dir,
            beats,
            notes,
        )
    } else {
        let clip_count = input.media_pool.len();
        let has_log = input
            .media_pool
            .iter()
            .any(|m| m.intelligence.is_log_footage);
        let dir = format!(
            "Tailored {} cut across {} attached asset(s) guided by your brief and '{}' playbook, balancing opening hook speed, visual rhythm, and clean broadcast finishing.",
            aspect_label(input.aspect_preset),
            clip_count,
            input.brand.name
        );
        let beats = vec![
            CreativePlanBeat {
                title: "Hook & Opening Frame".to_string(),
                time_range: fmt_range(0),
                visual_direction: "Open on the strongest visual/spoken moment immediately with tight framing and zero dead lead-in.".to_string(),
                audio_and_captions: "Immediate audio entry with readable safe-zone captions.".to_string(),
            },
            CreativePlanBeat {
                title: "Momentum & Story Spine".to_string(),
                time_range: fmt_range(1),
                visual_direction: "Shape primary clips around the narrative arc requested in your prompt, inserting B-roll or reframing to prevent visual fatigue.".to_string(),
                audio_and_captions: "Clean dialog cadence with balanced music bed underneath.".to_string(),
            },
            CreativePlanBeat {
                title: "Visual Accent & Emphasis".to_string(),
                time_range: fmt_range(2),
                visual_direction: format!(
                    "{} Pair key moments with restrained brand-tokenized callouts.",
                    if has_log {
                        "Recover Log footage cleanly to Rec.709 before applying brand color."
                    } else {
                        "Apply cohesive brand color contrast across cuts."
                    }
                ),
                audio_and_captions: "Highlight key words/metrics without crowding the frame.".to_string(),
            },
            CreativePlanBeat {
                title: "Button & Final Polish".to_string(),
                time_range: fmt_range(3),
                visual_direction: "Resolve on a decisive visual beat and verify framing + safe zones via QA.".to_string(),
                audio_and_captions: "Smooth fade-out and broadcast loudness polish.".to_string(),
            },
        ];
        let notes = vec![
            format!("Canvas: {}", aspect_label(input.aspect_preset)),
            format!("Attached assets: {clip_count}"),
            format!("Brand: {}", input.brand.name),
            "Connect an OpenRouter API key in AI & Keys for full LLM creative planning".to_string(),
        ];
        (
            "Tailored Footage Cut (Local Draft)".to_string(),
            dir,
            beats,
            notes,
        )
    };

    CreativePlan {
        title,
        approach,
        creative_direction,
        target_duration_sec: duration,
        production_mode: if programmatic {
            "programmatic".to_string()
        } else {
            "footage_edit".to_string()
        },
        skills_consulted: all_skills,
        beats,
        style_notes,
        model_used: "local-draft-planner".to_string(),
        used_local_fallback: true,
    }
}

pub async fn generate_creative_plan(
    openrouter: &OpenRouterClient,
    advisor: &ModelAdvisor,
    input: &CreativePlanInput<'_>,
    cancel: &CancellationToken,
) -> Result<CreativePlan, AgentError> {
    if cancel.is_cancelled() {
        return Err(AgentError::Cancelled);
    }

    if !openrouter.is_configured() {
        return Ok(build_local_creative_plan(input));
    }

    let programmatic = !has_visual_media(input.media_pool);
    let media_total_sec: f64 = input
        .media_pool
        .iter()
        .filter(|m| matches!(m.kind, MediaKind::Video | MediaKind::Audio))
        .map(|m| m.intelligence.duration_sec.max(0.0))
        .sum();
    let fallback_duration = infer_target_duration_from_prompt(input.prompt, media_total_sec);

    let dir_rec = advisor
        .recommend_with_cancel(ModelRoleKind::DirectorMultimodal, Some(cancel))
        .await;
    if cancel.is_cancelled() {
        return Err(AgentError::Cancelled);
    }

    let primary_model = input
        .model_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| dir_rec.primary.clone());
    let fallbacks = dir_rec.fallbacks.clone();

    let supports_json_mode = advisor
        .model_supports_parameter_with_cancel(&primary_model, "response_format", Some(cancel))
        .await
        || advisor
            .model_supports_parameter_with_cancel(
                &primary_model,
                "structured_outputs",
                Some(cancel),
            )
            .await;
    let supports_reasoning = advisor
        .model_supports_parameter_with_cancel(&primary_model, "reasoning", Some(cancel))
        .await;

    let (available_skills, skills_brief) = build_skills_brief();
    let saved_asset_names: Vec<String> = input
        .saved_assets
        .iter()
        .take(24)
        .map(|a| format!("{} ({})", a.id, a.name))
        .collect();

    let system_prompt = format!(
        "You are Solaris Creative Director in Planning Mode.\n\
         Read the Solaris video-making skills below to ground your craft (hooks, pacing, visual storytelling, color, motion graphics, typography, safe zones, and sound). \
         Do NOT force a generic or rigid pre-made checklist — freely design the best custom video plan tailored specifically to the user's prompt, genre, mood, aspect ratio, brand playbook, and available media.\n\
         - If no video/image footage is attached, design a cohesive programmatic motion-graphics / visual storytelling film (G1 HTML/CSS/SVG/GSAP; T1 captions are generated only when transcribed narration audio exists) that brings the user's prompt to life.\n\
         - If raw clips, voiceover, music, or scripts ARE attached, design an editorial plan grounded in those assets (and omit techniques the user didn't ask for — e.g., don't force silence cuts or neon captions if the user wants a calm cinematic montage).\n\
         - If `previous_plan` and `refinement_note` are supplied, revise the plan cleanly according to the user's feedback.\n\n\
         Output ONLY a single valid raw JSON object (no markdown fences, no commentary) matching:\n\
         {{\n\
           \"title\": string (short evocative title for this video),\n\
           \"approach\": string (2-5 words naming the tailored editorial/visual style),\n\
           \"creative_direction\": string (2-3 crisp sentences describing the concept, visual look, pacing, and emotional arc),\n\
           \"target_duration_sec\": number (realistic duration in seconds matching prompt/footage),\n\
           \"production_mode\": \"programmatic\" | \"footage_edit\",\n\
           \"skills_consulted\": [string] (names of the Solaris skills you drew craft principles from),\n\
           \"beats\": [\n\
             {{\n\
               \"title\": string,\n\
               \"time_range\": string (e.g. \"0.0s-3.5s\"),\n\
               \"visual_direction\": string (concrete visual/framing/motion/diagram plan for this beat),\n\
               \"audio_and_captions\": string (spoken/caption/music/SFX intention for this beat, or \"\" if silent)\n\
             }}\n\
           ] (choose the right number of beats for this video, typically 3 to 7),\n\
           \"style_notes\": [string] (3 to 5 concise craft decisions: typography, color/look, motion feel, audio/pacing)\n\
         }}\n\n\
         # Solaris Video-Making Skills\n\n{skills_brief}"
    );

    let user_payload = json!({
        "prompt": input.prompt,
        "aspect": aspect_label(input.aspect_preset),
        "production_mode_detected": if programmatic { "programmatic" } else { "footage_edit" },
        "suggested_duration_sec": fallback_duration,
        "brand": {
            "id": input.brand.id,
            "name": input.brand.name,
            "tokens": input.brand.tokens,
            "pacing_rules": input.brand.pacing_rules,
            "brand_md": input.brand.brand_md.chars().take(1800).collect::<String>(),
        },
        "attached_media": media_summary_json(input.media_pool),
        "saved_gfx_assets": saved_asset_names,
        "refinement_note": input.refinement_note,
        "previous_plan": input.previous_plan,
    })
    .to_string();

    let mut messages = vec![
        ChatMessage::system_cached(system_prompt),
        ChatMessage::user_text(user_payload),
    ];

    let (model_field, models_field) = if supports_reasoning {
        (Some(primary_model.clone()), None)
    } else {
        route_models(&primary_model, &fallbacks)
    };

    for attempt in 0..2 {
        let max_tokens = if attempt == 0 {
            PLANNER_MAX_TOKENS_INITIAL
        } else {
            PLANNER_MAX_TOKENS_RETRY
        };
        let req = ChatRequest {
            model: model_field.clone(),
            models: models_field.clone(),
            messages: messages.clone(),
            temperature: Some(if attempt == 0 { 0.45 } else { 0.2 }),
            max_tokens: Some(max_tokens),
            response_format: supports_json_mode.then(|| json!({ "type": "json_object" })),
            reasoning: supports_reasoning.then(|| ReasoningConfig {
                effort: Some("low".to_string()),
                ..ReasoningConfig::default()
            }),
            stream: Some(true),
            provider: Some(ProviderPreferences {
                require_parameters: if supports_reasoning { None } else { Some(true) },
                allow_fallbacks: Some(true),
                ..ProviderPreferences::default()
            }),
            ..ChatRequest::default()
        };

        let resp = openrouter.chat_stream(&req, Some(cancel), |_| {}).await?;

        let raw_text = resp
            .choices
            .first()
            .and_then(|c| c.message.content.as_ref())
            .map(|c| c.as_text())
            .unwrap_or_default();
        let raw = raw_text.trim();
        let cleaned = strip_json_fence(raw);
        match serde_json::from_str::<CreativePlan>(cleaned) {
            Ok(parsed)
                if !parsed.beats.is_empty() || !parsed.creative_direction.trim().is_empty() =>
            {
                let served_model = resp.model.unwrap_or_else(|| primary_model.clone());
                return Ok(normalize_creative_plan(
                    parsed,
                    fallback_duration,
                    programmatic,
                    &available_skills,
                    &served_model,
                    false,
                ));
            }
            Ok(_) | Err(_) if attempt == 0 => {
                messages.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: Some(crate::openrouter::MessageContent::Text(if raw.is_empty() {
                        "{}".to_string()
                    } else {
                        truncate_msg(raw)
                    })),
                    tool_calls: None,
                    tool_call_id: None,
                    reasoning_details: None,
                });
                messages.push(ChatMessage::user_text(
                    "Your previous response was truncated or invalid JSON. Return ONLY a complete, compact raw JSON object with keys: title, approach, creative_direction, target_duration_sec, production_mode, skills_consulted, beats, style_notes.",
                ));
            }
            Ok(_) => {
                return Err(AgentError::Serialization(
                    "Planner returned an empty plan object".to_string(),
                ));
            }
            Err(err) => {
                return Err(AgentError::Serialization(format!(
                    "Planner returned malformed JSON: {err} (raw: {})",
                    truncate_msg(cleaned)
                )));
            }
        }
    }

    Err(AgentError::Serialization(
        "Planner failed to generate a valid creative plan".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_creative_plan_adapts_between_programmatic_and_footage_modes() {
        let brand = BrandPlaybook::solaris_default();
        let prog_input = CreativePlanInput {
            prompt: "Create a 20s 9:16 explainer on how Rust borrow checking prevents memory leaks",
            aspect_preset: AspectPreset::Vertical9x16,
            brand: &brand,
            media_pool: &[],
            saved_assets: &[],
            model_override: None,
            refinement_note: None,
            previous_plan: None,
        };
        let prog_plan = build_local_creative_plan(&prog_input);
        assert_eq!(prog_plan.production_mode, "programmatic");
        assert!((prog_plan.target_duration_sec - 20.0).abs() < 0.1);
        assert!(prog_plan.used_local_fallback);
        assert!(
            prog_plan
                .skills_consulted
                .iter()
                .any(|s| s == "director-core")
        );
        assert_eq!(prog_plan.beats.len(), 4);
        assert!(prog_plan.title.contains("Rust borrow checking"));

        let clip = MediaAsset {
            id: "m1".to_string(),
            name: "interview.mp4".to_string(),
            path: "/tmp/interview.mp4".to_string(),
            kind: MediaKind::Video,
            role_hint: "a_roll".to_string(),
            added_at: chrono::Utc::now(),
            intelligence: solaris_core_ir::MediaIntelligenceManifest {
                duration_sec: 14.0,
                is_log_footage: true,
                ..Default::default()
            },
        };
        let footage_input = CreativePlanInput {
            prompt: "Make a calm documentary teaser with atmospheric pacing",
            aspect_preset: AspectPreset::Landscape16x9,
            brand: &brand,
            media_pool: &[clip],
            saved_assets: &[],
            model_override: None,
            refinement_note: None,
            previous_plan: None,
        };
        let footage_plan = build_local_creative_plan(&footage_input);
        assert_eq!(footage_plan.production_mode, "footage_edit");
        assert!((footage_plan.target_duration_sec - 14.0).abs() < 0.1);
        assert!(
            footage_plan
                .beats
                .iter()
                .any(|b| b.visual_direction.contains("Log footage"))
        );
    }

    #[test]
    fn normalize_creative_plan_clamps_duration_and_defaults_metadata() {
        let raw = r#"```json
        {
            "title": "Kinetic Launch Film",
            "approach": "Minimalist Kinetic Typography",
            "creative_direction": "Fast rhythmic typography synced to sharp visual beats.",
            "target_duration_sec": 999.0,
            "beats": [
                {"title": "Reveal", "time_range": "0.0s-4.0s", "visual_direction": "Full-bleed headline.", "audio_and_captions": "Single crisp line."}
            ],
            "style_notes": ["High contrast", ""]
        }
        ```"#;
        let parsed: CreativePlan = serde_json::from_str(strip_json_fence(raw)).unwrap();
        let skills = vec!["director-core".to_string(), "captions-and-gfx".to_string()];
        let normalized = normalize_creative_plan(
            parsed,
            24.0,
            true,
            &skills,
            "google/gemini-4-hydrogen",
            false,
        );
        assert_eq!(normalized.target_duration_sec, 180.0);
        assert_eq!(normalized.production_mode, "programmatic");
        assert_eq!(normalized.skills_consulted, skills);
        assert_eq!(normalized.style_notes, vec!["High contrast".to_string()]);
        assert_eq!(normalized.model_used, "google/gemini-4-hydrogen");
        assert!(!normalized.used_local_fallback);
    }
}
