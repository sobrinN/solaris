use std::collections::{BTreeMap, HashMap, HashSet};
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
use super::vision::{CapturedClipFrame, VisionClipFix};

const MAX_REPAIR_CLIPS_PER_PASS: usize = 7;
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
    captured_frame: Option<CapturedClipFrame>,
}

pub(super) fn filter_remaining_issue_fixes(clip_fixes: &[VisionClipFix]) -> Vec<VisionClipFix> {
    clip_fixes
        .iter()
        .filter(|f| f.severity.eq_ignore_ascii_case("issue") && !f.clip_id.trim().is_empty())
        .cloned()
        .collect()
}

fn group_actionable_fixes(
    project: &Project,
    clip_fixes: &[VisionClipFix],
    captured_by_clip: &HashMap<String, CapturedClipFrame>,
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
            captured_frame: captured_by_clip.get(&cid).cloned(),
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
    repair_round: usize,
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

    let brand_palette_note = if brand.is_some_and(|b| b.lock_brand_palette) {
        "Keep strict Brand Playbook CSS variables (`--brand-bg`, `--brand-surface`, `--brand-ink`, `--brand-accent`, `--brand-accent-secondary`, `--brand-font-display`, `--brand-font-body`) because `lock_brand_palette` is enabled."
    } else {
        "Keep typography bound to `var(--brand-font-display, Sora)` / `var(--brand-font-body, Sora)` while preserving any custom scene colors."
    };

    let system_prompt = format!(
        "You are Solaris Visual QA Repair Specialist. Fix ONLY the reported visual defects on this single G1 HTML/CSS/SVG/GSAP scene while preserving its language, core message, timing (`data-duration`), and visual cohesion.\n\
         RULES:\n\
         - Fix every reported overlap, clipped label, edge cutoff, layout imbalance, bottom void, empty node ring, low-contrast disclaimer, or redundant copy.\n\
         - {layout_band_rule} When fixing a bottom-half void without T1 captions, switch the stage container to a 3-zone CSS Grid (`display:grid;grid-template-rows:auto 1fr auto;row-gap:2vh;align-items:center;padding:7vh 5vw 9vh;box-sizing:border-box;`) so Header, Hero (`1fr`), and Bottom Callout/Legend fill the frame evenly with `>= 18px` vertical clearance between `header`/`.sub` and `.hero`. NEVER force `height:100%` stretch on tall `.hero` cards or apply `scale > 1.0` to a full-height card (which pushes the card upward into `header`/`.sub`).\n\
         - GSAP ENTRANCE TIMING VS. EMPTY SPACE (CRITICAL): The attached screenshot was captured at `captured_local_time_sec` (local time inside `data-duration`). Whenever the screenshot shows empty space inside a card or a blank lower stage even though DOM elements exist in `current_inline_html` (output panels, checkmarks, pills, footers), inspect their GSAP entrance timestamps FIRST. If any core/footer element is still at `opacity: 0` at `captured_local_time_sec` (e.g. starting at `2.15s..2.75s` on a `4.0s` clip), tighten and overlap the GSAP timeline so ALL core + footer elements finish entering by `<= 45%` of `duration_sec` (max `~1.8s`) instead of stretching the card height or adding redundant DOM blocks.\n\
         - CSS SCOPING FOR BACKGROUND SVGs (CRITICAL): NEVER write a descendant selector like `.funnel svg {{ position: absolute; inset: 0; width: 100%; height: 100%; }}` when nested icon `<svg>` elements exist inside that container — GSAP transforms make child cards containing blocks and stretch every icon `<svg>` directly over the card text! ALWAYS target backdrop SVGs with a direct-child selector (`.funnel > svg`) or an explicit class (`.funnel-bg`).\n\
         - FLOW-POSITIONED BADGES & SEALS: NEVER hang a circular seal or badge off a giant headline with `position: absolute` and negative `right`/`top` offsets in 9:16 portrait — place badges in normal Flex/Grid flow (above the headline in the eyebrow row or inline with explicit `gap`) so headline-badge collision is impossible. Keep decorative rings/radars behind foreground pills subtle (`opacity <= 0.35`) and remove high-contrast accent dots that touch pill/card edges.\n\
         - SVG & DIAGRAM ANTI-COLLISION & CENTERING: NEVER leave a cyan/accent dot or circle sitting on top of text (like a central panel title). Move multi-word labels, risk cards, or callouts into clean HTML CSS Flex/Grid containers rather than cramped `<text x y>` over dense SVG paths. Keep short SVG labels cleanly outside node radii (`>= 16px` clearance), center hero diagrams symmetrically at `(W/2, H/2)`, and keep `>= 32px` inner padding inside enclosing shields/rings so labels never cross outlines. Whenever GSAP animates `rotation` or `scale` on an SVG element, ALWAYS specify explicit `svgOrigin: \"<cx> <cy>\"` matching the `viewBox` center so rotating globes/rings never swing off-center. Populate any empty placeholder node circle with a crisp linear icon or solid glowing core dot.\n\
         - REMOVE any unsolicited scene/step counter badges (`01 / 06`, `02/06`, `ETAPA 02`, `PASSO 1`, `STEP 2`, `Scene 1/6`) and REMOVE any useless bottom loading/progress bars unless the defect explicitly asks to keep them.\n\
         - Keep at least `16px` (`3vw`) padding inside cards/viewport so zero characters clip, keep secondary footers/disclaimers `>= 13px` at `>= 0.70` ink opacity, and never let absolute-positioned badges overlap captions or headlines.\n\
         - {brand_palette_note}\n\
         - Keep strict Solaris sandbox rules: root `<div class=\"clip\" data-start=\"0\" data-duration=\"<duration_sec from user JSON>\">`, full-canvas `position:fixed;inset:0;width:100vw;height:100vh;overflow:hidden;`, inline GSAP (`window.gsap`) ONLY, ZERO `<script src=...>`, ZERO `<link>`, ZERO external URLs/CDNs, ZERO `Math.random`/`Date.now`/`setInterval`/`setTimeout`/`requestAnimationFrame`/`fetch`.\n\
         - Return ONLY a valid JSON object: {{\"inline_html\": \"<complete repaired HTML>\"}} (no markdown fences, no commentary)."
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

    let mut payload_obj = json!({
        "clip_id": plan.clip.id,
        "label": plan.clip.label,
        "duration_sec": plan.clip.duration_sec,
        "captured_local_time_sec": plan.captured_frame.as_ref().map(|f| f.local_time_sec),
        "canvas": project.canvas,
        "has_active_captions": has_captions,
        "brand_tokens": brand.map(|b| &b.tokens),
        "repair_round": repair_round,
        "defects_to_fix": defects_json,
        "current_inline_html": plan.current_html,
    });
    if repair_round >= 2
        && let Some(obj) = payload_obj.as_object_mut()
    {
        obj.insert(
            "escalation_note".to_string(),
            json!("RETRY ROUND 2 (CRITICAL): A previous repair pass STILL left the defect(s) shown in the attached screenshot. Do NOT make tiny pixel nudges (such as shifting right:-14vw to right:-17vw). Apply a structural CSS/HTML/GSAP fix: pull any late GSAP entrance start times earlier so all core and footer elements are visible by <= 1.6s, remove height:100% / scale > 1 on overflowing cards, convert any colliding position:absolute badge/seal into normal CSS Flex/Grid flow, replace any descendant .container svg backdrop selector with .container > svg or an explicit class, and remove or fade decorative SVG dots that touch card/pill edges."),
        );
    }
    let payload = payload_obj.to_string();

    let mut parts = vec![ContentPart::text(payload)];
    if let Some(ref frame) = plan.captured_frame {
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
        session_id: Some(format!("solaris-qa-repair-{}-{}", project.id, plan.clip.id)),
        provider: Some(ProviderPreferences {
            allow_fallbacks: Some(true),
            ..ProviderPreferences::default()
        }),
        reasoning: supports_reasoning.then(|| ReasoningConfig {
            effort: Some("medium".to_string()),
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
    captured_by_clip: &HashMap<String, CapturedClipFrame>,
    repair_round: usize,
    cancel: &CancellationToken,
    ops_counter: Option<&AtomicU32>,
    already_repaired: &mut HashSet<String>,
    auto_fixes_applied: &mut u32,
    checks: &mut Vec<QaCheckItem>,
) -> Vec<String> {
    if cancel.is_cancelled() || !openrouter.is_configured() || clip_fixes.is_empty() {
        return Vec::new();
    }

    let plans = group_actionable_fixes(project, clip_fixes, captured_by_clip);
    if plans.is_empty() {
        return Vec::new();
    }

    let _ = backend
        .emit_cursor_pulse(
            &project.id,
            "director-qa",
            AgentRole::Director,
            "timeline.track.g1",
            &format!(
                "repairing {} GFX scene(s) (pass {repair_round})",
                plans.len()
            ),
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
            repair_round,
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
        if !lint.valid || lint.descendant_svg_trap {
            let reason_details = if !lint.valid {
                lint.errors.join("; ")
            } else {
                lint.warnings
                    .iter()
                    .filter(|w| w.contains("Descendant CSS selector"))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; ")
            };
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
                details: Some(reason_details),
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
        return Vec::new();
    }

    let summary = if ready_plans.len() == 1 {
        format!("QA repaired GFX '{}'", ready_plans[0].clip.label)
    } else {
        format!("QA repaired {} GFX scenes", ready_plans.len())
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
        return Vec::new();
    }

    let mut repaired_ids = Vec::with_capacity(ready_plans.len());
    for plan in ready_plans {
        if already_repaired.insert(plan.clip.id.clone()) {
            *auto_fixes_applied += 1;
        }
        repaired_ids.push(plan.clip.id.clone());
        let first_defect = plan
            .defects
            .first()
            .map(|d| d.defect.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or("visual layout defect");
        let pass_suffix = if repair_round > 1 {
            format!(" (pass {repair_round})")
        } else {
            String::new()
        };
        checks.push(QaCheckItem {
            id: format!("vision-repaired-r{repair_round}-{}", plan.clip.id),
            category: "visual".to_string(),
            severity: QaSeverity::Fixed,
            message: format!(
                "Auto-repaired GFX '{}' ({}){pass_suffix}: addressed '{}'.",
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
                "Vision QA auto-repaired GFX '{}' ({}){pass_suffix}",
                plan.clip.label, plan.clip.id
            ),
            details: Some(first_defect.to_string()),
        });
    }

    repaired_ids
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

        let remaining_issues = filter_remaining_issue_fixes(&fixes);
        assert_eq!(remaining_issues.len(), 2);
        assert!(remaining_issues.iter().all(|f| f.severity == "issue"));
        assert!(!remaining_issues.iter().any(|f| f.clip_id == "s1"));
    }
}
