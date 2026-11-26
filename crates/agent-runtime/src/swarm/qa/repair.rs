use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::AtomicU32;

use chrono::Utc;
use futures_util::future::join_all;
use serde::Deserialize;
use serde_json::json;
use solaris_core_ir::{
    AgentRole, BrandPlaybook, Clip, ClipKind, LogEntry, LogLevel, Project, QaCheckItem, QaSeverity,
    TimelineCommand,
};
use tokio_util::sync::CancellationToken;

use crate::backend::SolarisBackend;
use crate::lint::lint_gfx_html;
use crate::openrouter::{
    ChatMessage, ChatRequest, ContentPart, MediaUrlRef, OpenRouterClient, ProviderPreferences,
    ReasoningConfig,
};
use crate::swarm::llm::{route_models, strip_json_fence};

use super::apply_locked_op;
use super::vision::VisionClipFix;

const MAX_REPAIR_CLIPS_PER_PASS: usize = 4;
const REPAIR_MAX_TOKENS: u32 = 8192;

#[derive(Debug, Deserialize)]
struct RepairedGfxPayload {
    #[serde(default, alias = "html", alias = "html_source")]
    inline_html: String,
}

#[derive(Debug, Clone)]
struct ClipRepairPlan {
    clip: Clip,
    current_html: String,
    defects: Vec<VisionClipFix>,
    frame_png_b64: Option<String>,
}

fn group_actionable_fixes(
    project: &Project,
    clip_fixes: &[VisionClipFix],
    captured_by_clip: &HashMap<String, String>,
) -> Vec<ClipRepairPlan> {
    let Some(g1) = project
        .tracks
        .iter()
        .find(|t| t.id == "g1" && !t.muted && !t.locked)
    else {
        return Vec::new();
    };

    let mut by_clip: BTreeMap<String, Vec<VisionClipFix>> = BTreeMap::new();
    for fix in clip_fixes {
        let cid = fix.clip_id.trim();
        if cid.is_empty() {
            continue;
        }
        by_clip
            .entry(cid.to_string())
            .or_default()
            .push(fix.clone());
    }

    let mut plans = Vec::new();
    for (cid, defects) in by_clip {
        let Some(clip) = g1.clips.iter().find(|c| c.id == cid) else {
            continue;
        };
        let ClipKind::Gfx { gfx } = &clip.payload else {
            continue;
        };
        let Some(html) = gfx
            .inline_html
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        plans.push(ClipRepairPlan {
            clip: clip.clone(),
            current_html: html.to_string(),
            defects,
            frame_png_b64: captured_by_clip.get(&cid).cloned(),
        });
    }

    // Prioritize clips with hard "issue" severity first, then by timeline order.
    plans.sort_by(|a, b| {
        let a_has_issue = a
            .defects
            .iter()
            .any(|d| d.severity.eq_ignore_ascii_case("issue"));
        let b_has_issue = b
            .defects
            .iter()
            .any(|d| d.severity.eq_ignore_ascii_case("issue"));
        b_has_issue
            .cmp(&a_has_issue)
            .then_with(|| a.clip.start_sec.total_cmp(&b.clip.start_sec))
    });
    plans.truncate(MAX_REPAIR_CLIPS_PER_PASS);
    plans
}

pub(super) fn extract_repaired_html(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    // 1. JSON payload (`{"inline_html": "..."}` or fenced ```json)
    let json_candidate = strip_json_fence(trimmed);
    if let Ok(parsed) = serde_json::from_str::<RepairedGfxPayload>(json_candidate) {
        let html = parsed.inline_html.trim();
        if !html.is_empty() {
            return Some(html.to_string());
        }
    }

    // 2. Fenced ```html block
    if let Some(start) = trimmed.find("```html") {
        let after = &trimmed[start + "```html".len()..];
        let body = after
            .find("```")
            .map(|end| &after[..end])
            .unwrap_or(after)
            .trim();
        if !body.is_empty() {
            return Some(body.to_string());
        }
    }

    // 3. Raw HTML fallback — slice from the earliest <style>/<div tag through the last
    // closing </div>/</script> so leading or trailing LLM prose never leaks into frames.
    let start_idx = [trimmed.find("<style"), trimmed.find("<div")]
        .into_iter()
        .flatten()
        .min();
    if let Some(start) = start_idx {
        let from_tag = &trimmed[start..];
        let end_idx = [
            from_tag.rfind("</div>").map(|i| i + "</div>".len()),
            from_tag.rfind("</script>").map(|i| i + "</script>".len()),
            from_tag.rfind("</style>").map(|i| i + "</style>".len()),
        ]
        .into_iter()
        .flatten()
        .max();
        let candidate = end_idx
            .map(|end| &from_tag[..end])
            .unwrap_or(from_tag)
            .trim();
        if !candidate.is_empty() {
            return Some(candidate.to_string());
        }
    }

    None
}

#[allow(clippy::too_many_arguments)]
async fn request_repaired_html(
    openrouter: &OpenRouterClient,
    model: &str,
    fallbacks: &[String],
    supports_reasoning: bool,
    project: &Project,
    brand: Option<&BrandPlaybook>,
    plan: &ClipRepairPlan,
    cancel: &CancellationToken,
) -> Result<String, String> {
    let is_portrait = project.canvas.height > project.canvas.width;
    let has_captions = project
        .tracks
        .iter()
        .any(|t| t.id == "t1" && !t.muted && !t.clips.is_empty());
    let layout_band_rule = match (is_portrait, has_captions) {
        (true, true) => {
            "VERTICAL 9:16 WITH T1 CAPTIONS ACTIVE: Keep ALL foreground content cleanly inside `6vh..62vh` so `64vh..76vh` stays clear for T1 captions, while extending ambient backdrop across `100vh`."
        }
        (true, false) => {
            "VERTICAL 9:16 WITHOUT T1 CAPTIONS: Distribute foreground cleanly across `8vh..84vh` with rich `100vh` ambient backdrop and zero bottom-half dead void."
        }
        (false, true) => {
            "WIDESCREEN/SQUARE WITH T1 CAPTIONS ACTIVE: Keep foreground cleanly inside `6vh..72vh` so `75vh..88vh` stays clear for T1 captions."
        }
        (false, false) => {
            "WIDESCREEN/SQUARE WITHOUT T1 CAPTIONS: Balance foreground across `8vh..88vh` with zero dead voids."
        }
    };

    let system_prompt = format!(
        "You are Solaris Visual QA Repair Specialist. Fix ONLY the reported visual defects on this single G1 HTML/CSS/SVG/GSAP scene while preserving its language, core message, timing (`data-duration`), and visual cohesion.\n\
         RULES:\n\
         - Fix every reported overlap, clipped label, edge cutoff, layout imbalance, or redundant copy.\n\
         - {layout_band_rule}\n\
         - REMOVE any unsolicited scene counter badges (`01 / 06`, `02/06`, `Scene 1/6`) and REMOVE any useless bottom loading/progress bars unless the defect explicitly asks to keep them.\n\
         - Keep at least `16px` (`3vw`) padding inside cards/viewport so zero characters clip, and never let absolute-positioned badges overlap captions or headlines.\n\
         - Keep strict Solaris sandbox rules: root `<div class=\"clip\" data-start=\"0\" data-duration=\"{:.2}\">`, full-canvas `position:fixed;inset:0;width:100vw;height:100vh;overflow:hidden;`, inline GSAP (`window.gsap`) ONLY, ZERO `<script src=...>`, ZERO `<link>`, ZERO external URLs/CDNs, ZERO `Math.random`/`Date.now`/`setInterval`/`setTimeout`/`requestAnimationFrame`/`fetch`.\n\
         - Return ONLY a valid JSON object: {{\"inline_html\": \"<complete repaired HTML>\"}} (no markdown fences, no commentary).",
        plan.clip.duration_sec
    );

    let defects_json: Vec<serde_json::Value> = plan
        .defects
        .iter()
        .map(|d| {
            json!({
                "severity": d.severity,
                "defect": d.defect,
                "fix_instruction": d.fix_instruction,
            })
        })
        .collect();

    let payload = json!({
        "clip_id": plan.clip.id,
        "label": plan.clip.label,
        "duration_sec": plan.clip.duration_sec,
        "canvas": project.canvas,
        "has_active_captions": has_captions,
        "brand_tokens": brand.map(|b| &b.tokens),
        "defects_to_fix": defects_json,
        "current_inline_html": plan.current_html,
    })
    .to_string();

    let mut parts = vec![ContentPart::text(payload)];
    if let Some(ref b64) = plan.frame_png_b64 {
        parts.push(ContentPart::ImageUrl {
            image_url: MediaUrlRef {
                url: format!("data:image/png;base64,{b64}"),
            },
        });
    }

    let (req_model, req_models) = route_models(model, fallbacks);
    let req = ChatRequest {
        model: req_model,
        models: req_models,
        messages: vec![
            ChatMessage::system(system_prompt),
            ChatMessage::user_parts(parts),
        ],
        session_id: Some(format!("solaris-qa-repair-{}-{}", project.id, plan.clip.id)),
        provider: Some(ProviderPreferences {
            allow_fallbacks: Some(true),
            ..ProviderPreferences::default()
        }),
        reasoning: supports_reasoning.then(|| ReasoningConfig {
            effort: Some("low".to_string()),
            ..ReasoningConfig::default()
        }),
        max_tokens: Some(REPAIR_MAX_TOKENS),
        temperature: Some(0.15),
        ..ChatRequest::default()
    };

    let resp = openrouter
        .chat_stream(&req, Some(cancel), |_| {})
        .await
        .map_err(|err| super::vision::scrub_secrets(&err.to_string()))?;
    let text = resp
        .choices
        .first()
        .and_then(|c| c.message.content.as_ref())
        .map(|c| c.as_text())
        .unwrap_or_default();
    extract_repaired_html(&text)
        .ok_or_else(|| "repaired HTML could not be extracted from model response".to_string())
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn repair_gfx_clips_from_vision(
    backend: &dyn SolarisBackend,
    openrouter: &OpenRouterClient,
    vision_model: (&str, &[String]),
    supports_reasoning: bool,
    project: &Project,
    brand: Option<&BrandPlaybook>,
    clip_fixes: &[VisionClipFix],
    captured_by_clip: &HashMap<String, String>,
    cancel: &CancellationToken,
    ops_counter: Option<&AtomicU32>,
    auto_fixes_applied: &mut u32,
    checks: &mut Vec<QaCheckItem>,
) -> u32 {
    if cancel.is_cancelled() || !openrouter.is_configured() || clip_fixes.is_empty() {
        return 0;
    }

    let plans = group_actionable_fixes(project, clip_fixes, captured_by_clip);
    if plans.is_empty() {
        return 0;
    }

    let _ = backend
        .emit_cursor_pulse(
            &project.id,
            "director-qa",
            AgentRole::Director,
            "timeline.track.g1",
            &format!("enhancing {} GFX scene(s)", plans.len()),
        )
        .await;

    let (model, fallbacks) = vision_model;
    let repair_futures = plans.iter().map(|plan| {
        request_repaired_html(
            openrouter,
            model,
            fallbacks,
            supports_reasoning,
            project,
            brand,
            plan,
            cancel,
        )
    });
    let repaired_results = join_all(repair_futures).await;

    let mut batch_commands = Vec::new();
    let mut ready_plans = Vec::new();

    for (plan, candidate_res) in plans.into_iter().zip(repaired_results) {
        if cancel.is_cancelled() {
            break;
        }
        let new_html = match candidate_res {
            Ok(html) => html,
            Err(err_msg) => {
                backend.emit_log(LogEntry {
                    id: uuid::Uuid::new_v4().to_string(),
                    timestamp: Utc::now(),
                    level: LogLevel::Warn,
                    source: "qa".to_string(),
                    project_id: Some(project.id.clone()),
                    run_id: None,
                    agent_id: Some("director-qa".to_string()),
                    message: format!(
                        "Vision QA repair failed for '{}' ({})",
                        plan.clip.label, plan.clip.id
                    ),
                    details: Some(err_msg),
                });
                continue;
            }
        };
        if new_html.trim() == plan.current_html.trim() {
            continue;
        }
        let lint = lint_gfx_html(&new_html);
        if !lint.valid {
            backend.emit_log(LogEntry {
                id: uuid::Uuid::new_v4().to_string(),
                timestamp: Utc::now(),
                level: LogLevel::Warn,
                source: "qa".to_string(),
                project_id: Some(project.id.clone()),
                run_id: None,
                agent_id: Some("director-qa".to_string()),
                message: format!(
                    "Skipped Vision QA repair for '{}' ({}): repaired HTML failed lint",
                    plan.clip.label, plan.clip.id
                ),
                details: Some(lint.errors.join("; ")),
            });
            continue;
        }

        let ClipKind::Gfx { mut gfx } = plan.clip.payload.clone() else {
            continue;
        };
        gfx.inline_html = Some(new_html);

        batch_commands.push(TimelineCommand::DeleteClip {
            track_id: "g1".to_string(),
            clip_id: plan.clip.id.clone(),
        });
        batch_commands.push(TimelineCommand::PlaceGfx {
            track_id: "g1".to_string(),
            clip_id: plan.clip.id.clone(),
            label: plan.clip.label.clone(),
            start_sec: plan.clip.start_sec,
            duration_sec: plan.clip.duration_sec,
            gfx,
        });
        batch_commands.push(TimelineCommand::UpdateClipTransform {
            track_id: "g1".to_string(),
            clip_id: plan.clip.id.clone(),
            transform: plan.clip.transform.clone(),
        });
        ready_plans.push(plan);
    }

    if ready_plans.is_empty() || cancel.is_cancelled() {
        return 0;
    }

    let summary = if ready_plans.len() == 1 {
        format!("QA enhanced GFX '{}'", ready_plans[0].clip.label)
    } else {
        format!("QA enhanced {} GFX scenes", ready_plans.len())
    };
    let batch = TimelineCommand::Batch {
        commands: batch_commands,
        summary: Some(summary),
    };

    if !apply_locked_op(
        backend,
        &project.id,
        "g1",
        batch,
        "Vision QA visual auto-repair",
        ops_counter,
    )
    .await
    {
        return 0;
    }

    let repaired_count = ready_plans.len() as u32;
    *auto_fixes_applied += repaired_count;
    for plan in ready_plans {
        let first_defect = plan
            .defects
            .first()
            .map(|d| d.defect.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or("visual layout defect");
        checks.push(QaCheckItem {
            id: format!("vision-repaired-{}", plan.clip.id),
            category: "visual".to_string(),
            severity: QaSeverity::Fixed,
            message: format!(
                "Auto-repaired GFX '{}' ({}): addressed '{}'.",
                plan.clip.label, plan.clip.id, first_defect
            ),
            time_sec: Some(plan.clip.start_sec),
            auto_fixed: true,
        });
        backend.emit_log(LogEntry {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            level: LogLevel::Info,
            source: "qa".to_string(),
            project_id: Some(project.id.clone()),
            run_id: None,
            agent_id: Some("director-qa".to_string()),
            message: format!(
                "Vision QA auto-repaired GFX '{}' ({})",
                plan.clip.label, plan.clip.id
            ),
            details: Some(first_defect.to_string()),
        });
    }

    repaired_count
}

#[cfg(test)]
mod tests {
    use super::*;
    use solaris_core_ir::{AspectPreset, AudioSettings, GfxInstance, Transform2D};

    #[test]
    fn extract_repaired_html_parses_json_fenced_html_and_raw_html() {
        let json_raw = r#"{"inline_html": "<div class=\"clip\" data-start=\"0\" data-duration=\"4\">Fixed</div>"}"#;
        assert_eq!(
            extract_repaired_html(json_raw).as_deref(),
            Some(r#"<div class="clip" data-start="0" data-duration="4">Fixed</div>"#)
        );

        let fenced =
            "```html\n<div class=\"clip\" data-start=\"0\" data-duration=\"4\">Hi</div>\n```";
        assert_eq!(
            extract_repaired_html(fenced).as_deref(),
            Some(r#"<div class="clip" data-start="0" data-duration="4">Hi</div>"#)
        );

        let prose_wrapped = "Here is the repaired HTML:\n<style>.x{color:#fff}</style><div class=\"clip\" data-start=\"0\" data-duration=\"4\">Clean</div>\nHope this helps!";
        assert_eq!(
            extract_repaired_html(prose_wrapped).as_deref(),
            Some(
                r#"<style>.x{color:#fff}</style><div class="clip" data-start="0" data-duration="4">Clean</div>"#
            )
        );

        assert!(extract_repaired_html("   ").is_none());
    }

    #[test]
    fn group_actionable_fixes_prioritizes_issues_and_ignores_unknown_clips() {
        let mut proj = Project::new_default("repair_group", AspectPreset::Vertical9x16);
        let g1 = proj.tracks.iter_mut().find(|t| t.id == "g1").unwrap();
        for (id, start) in [("s1", 0.0), ("s2", 4.0)] {
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
                        inline_html: Some(
                            r#"<div class="clip" data-start="0" data-duration="4">Scene</div>"#
                                .into(),
                        ),
                    },
                },
            });
        }
        let fixes = vec![
            VisionClipFix {
                clip_id: "unknown".into(),
                severity: "issue".into(),
                defect: "Ghost".into(),
                fix_instruction: "Ignore".into(),
            },
            VisionClipFix {
                clip_id: "s1".into(),
                severity: "warning".into(),
                defect: "Progress bar".into(),
                fix_instruction: "Remove progress bar".into(),
            },
            VisionClipFix {
                clip_id: "s2".into(),
                severity: "issue".into(),
                defect: "Clipped label".into(),
                fix_instruction: "Add padding".into(),
            },
        ];
        let plans = group_actionable_fixes(&proj, &fixes, &HashMap::new());
        assert_eq!(plans.len(), 2);
        assert_eq!(
            plans[0].clip.id, "s2",
            "issue clip must be prioritized first"
        );
        assert_eq!(plans[1].clip.id, "s1");
    }
}
