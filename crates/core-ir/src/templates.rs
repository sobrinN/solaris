use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::commands::{AgentRole, CommandMeta, TimelineCommand};
use crate::engine::ProjectEngine;
use crate::model::{AspectPreset, ColorGrade, GfxInstance, Project, SavedCodeAsset};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectTemplateInfo {
    pub id: String,
    pub name: String,
    pub category: String,
    pub description: String,
    pub aspect_preset: AspectPreset,
    pub recommended_prompt: String,
    pub default_duration_sec: f64,
    pub tags: Vec<String>,
}

pub fn builtin_templates() -> Vec<ProjectTemplateInfo> {
    vec![
        ProjectTemplateInfo {
            id: "talking-head-short".to_string(),
            name: "Talking-Head Short".to_string(),
            category: "short_form".to_string(),
            description: "9:16 vertical reel scaffold with 2.5s hook title, mid-video stat card, and Rec.709 punchy grade (captions come only from transcribed speech).".to_string(),
            aspect_preset: AspectPreset::Vertical9x16,
            recommended_prompt: "Cut silences with 0.12s breath pad, add a 1.08x hook zoom, synced captions from spoken audio, and -14 LUFS ducked mix.".to_string(),
            default_duration_sec: 18.0,
            tags: vec![
                "9:16".to_string(),
                "reels".to_string(),
                "talking-head".to_string(),
                "captions".to_string(),
            ],
        },
        ProjectTemplateInfo {
            id: "faceless-explainer".to_string(),
            name: "Faceless Explainer".to_string(),
            category: "explainer".to_string(),
            description: "16:9 widescreen explainer scaffold with 3-chapter GFX beats, lower-third callout, and cinematic contrast (captions come only from transcribed voiceover).".to_string(),
            aspect_preset: AspectPreset::Landscape16x9,
            recommended_prompt: "Assemble voiceover on A1, cover with Ken Burns B-roll on V2, place 3-chapter GFX cards, and normalize to -14 LUFS.".to_string(),
            default_duration_sec: 24.0,
            tags: vec![
                "16:9".to_string(),
                "youtube".to_string(),
                "explainer".to_string(),
                "gfx".to_string(),
            ],
        },
    ]
}

pub fn get_template(id: &str) -> Option<ProjectTemplateInfo> {
    builtin_templates().into_iter().find(|t| t.id == id)
}

fn find_asset_html(assets: &[SavedCodeAsset], asset_id: &str) -> Option<String> {
    if let Some(found) = assets.iter().find(|a| a.id == asset_id) {
        return Some(found.html_source.clone());
    }
    let fallback = match asset_id {
        "chapter-title-solar" => {
            r#"<div class="clip" data-start="0" data-duration="2.5" style="display:grid;place-items:center;font-family:var(--brand-font-display,Sora);color:var(--brand-ink,#F7F5EF);"><div style="padding:14px 24px;border-radius:16px;background:rgba(26,14,14,0.86);border:1px solid rgba(240,90,0,0.45);text-align:center;"><div style="font-size:11px;letter-spacing:0.2em;color:var(--brand-accent-secondary,#F7B733);">{{eyebrow}}</div><div style="font-size:28px;font-weight:600;margin-top:4px;">{{headline}}</div></div></div>"#
        }
        "stat-card-solar" => {
            r#"<div class="clip" data-start="0" data-duration="3.0" style="display:flex;align-items:center;gap:12px;padding:14px 20px;border-radius:16px;background:rgba(8,4,4,0.88);border:1px solid rgba(247,245,239,0.16);color:#F7F5EF;font-family:var(--brand-font-display,Sora);"><span style="font-size:26px;font-weight:700;color:var(--brand-accent,#F05A00);">{{value}}</span><span style="font-size:12px;letter-spacing:0.08em;opacity:0.85;">{{label}}</span></div>"#
        }
        "comment-bubble-solar" => {
            r#"<div class="clip" data-start="0" data-duration="2.8" style="padding:12px 16px;border-radius:18px;background:rgba(26,14,14,0.92);border:1px solid rgba(247,183,51,0.35);color:#F7F5EF;font-family:var(--brand-font-body,Sora);"><div style="font-size:11px;color:#F7B733;font-weight:600;">{{author}}</div><div style="font-size:14px;margin-top:2px;">{{comment}}</div></div>"#
        }
        "lower-third-solar" => {
            r#"<div class="clip" data-start="0" data-duration="3.2" style="display:flex;align-items:center;gap:10px;padding:10px 16px;border-radius:14px;background:rgba(8,4,4,0.88);border-left:3px solid var(--brand-accent,#F05A00);color:var(--brand-ink,#F7F5EF);font-family:var(--brand-font-display,Sora);"><div><div style="font-size:15px;font-weight:600;">{{title}}</div><div style="font-size:11px;color:var(--brand-accent-secondary,#F7B733);">{{subtitle}}</div></div></div>"#
        }
        _ => return None,
    };
    Some(fallback.to_string())
}

pub fn instantiate_template(
    template_id: &str,
    name: Option<String>,
    brand_playbook_id: Option<String>,
    prompt_override: Option<String>,
    aspect_override: Option<AspectPreset>,
    assets: &[SavedCodeAsset],
) -> Option<Project> {
    let info = get_template(template_id)?;
    let mut project = Project::new_default(
        name.filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| info.name.clone()),
        aspect_override.unwrap_or(info.aspect_preset),
    );
    if let Some(bid) = brand_playbook_id.filter(|b| !b.trim().is_empty()) {
        project.brand_playbook_id = bid;
    }
    project.prompt = prompt_override
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| info.recommended_prompt.clone());

    let mut engine = ProjectEngine::new(project);
    let meta = CommandMeta {
        agent_id: "template-init".to_string(),
        role: AgentRole::Director,
        reason: Some(format!("instantiate template {template_id}")),
    };

    let commands = match template_id {
        "talking-head-short" => build_talking_head_commands(assets),
        "faceless-explainer" => build_faceless_explainer_commands(assets),
        _ => return None,
    };

    for cmd in commands {
        engine.apply(cmd, meta.clone()).ok()?;
    }

    Some(engine.project().clone())
}

fn build_talking_head_commands(assets: &[SavedCodeAsset]) -> Vec<TimelineCommand> {
    vec![
        TimelineCommand::SetColorGrade {
            grade: ColorGrade {
                contrast: 1.12,
                saturation: 1.08,
                temp: 0.03,
                ..ColorGrade::default()
            },
            track_id: None,
            clip_id: None,
        },
        TimelineCommand::PlaceGfx {
            track_id: "g1".to_string(),
            clip_id: "tpl-gfx-hook".to_string(),
            label: "Hook Chapter Title".to_string(),
            start_sec: 0.0,
            duration_sec: 2.5,
            gfx: GfxInstance {
                asset_id: Some("chapter-title-solar".to_string()),
                props: json!({ "eyebrow": "THE HOOK", "headline": "3 KEY SHIFTS" }),
                inline_html: find_asset_html(assets, "chapter-title-solar"),
            },
        },
        TimelineCommand::PlaceGfx {
            track_id: "g1".to_string(),
            clip_id: "tpl-gfx-stat".to_string(),
            label: "Mid Stat Card".to_string(),
            start_sec: 6.5,
            duration_sec: 3.0,
            gfx: GfxInstance {
                asset_id: Some("stat-card-solar".to_string()),
                props: json!({ "value": "10X", "label": "FASTER WORKFLOW" }),
                inline_html: find_asset_html(assets, "stat-card-solar"),
            },
        },
        TimelineCommand::PlaceGfx {
            track_id: "g1".to_string(),
            clip_id: "tpl-gfx-cta".to_string(),
            label: "Outro CTA Bubble".to_string(),
            start_sec: 14.5,
            duration_sec: 2.8,
            gfx: GfxInstance {
                asset_id: Some("comment-bubble-solar".to_string()),
                props: json!({ "author": "@solaris", "comment": "Save this for your next edit" }),
                inline_html: find_asset_html(assets, "comment-bubble-solar"),
            },
        },
    ]
}

fn build_faceless_explainer_commands(assets: &[SavedCodeAsset]) -> Vec<TimelineCommand> {
    vec![
        TimelineCommand::SetColorGrade {
            grade: ColorGrade {
                contrast: 1.15,
                saturation: 1.05,
                exposure: 0.05,
                ..ColorGrade::default()
            },
            track_id: None,
            clip_id: None,
        },
        TimelineCommand::PlaceGfx {
            track_id: "g1".to_string(),
            clip_id: "tpl-exp-ch1".to_string(),
            label: "Chapter 01".to_string(),
            start_sec: 0.0,
            duration_sec: 2.5,
            gfx: GfxInstance {
                asset_id: Some("chapter-title-solar".to_string()),
                props: json!({ "eyebrow": "CHAPTER 01", "headline": "THE PROBLEM" }),
                inline_html: find_asset_html(assets, "chapter-title-solar"),
            },
        },
        TimelineCommand::PlaceGfx {
            track_id: "g1".to_string(),
            clip_id: "tpl-exp-lt".to_string(),
            label: "Speaker / Topic Lower Third".to_string(),
            start_sec: 5.0,
            duration_sec: 3.2,
            gfx: GfxInstance {
                asset_id: Some("lower-third-solar".to_string()),
                props: json!({ "title": "Core Architecture", "subtitle": "Local-First Pipeline" }),
                inline_html: find_asset_html(assets, "lower-third-solar"),
            },
        },
        TimelineCommand::PlaceGfx {
            track_id: "g1".to_string(),
            clip_id: "tpl-exp-stat".to_string(),
            label: "Key Metric Callout".to_string(),
            start_sec: 11.5,
            duration_sec: 3.0,
            gfx: GfxInstance {
                asset_id: Some("stat-card-solar".to_string()),
                props: json!({ "value": "-14 LUFS", "label": "BROADCAST LOUDNESS" }),
                inline_html: find_asset_html(assets, "stat-card-solar"),
            },
        },
        TimelineCommand::PlaceGfx {
            track_id: "g1".to_string(),
            clip_id: "tpl-exp-cta".to_string(),
            label: "Closing Callout".to_string(),
            start_sec: 19.5,
            duration_sec: 3.0,
            gfx: GfxInstance {
                asset_id: Some("comment-bubble-solar".to_string()),
                props: json!({ "author": "Next Step", "comment": "Drop your footage to begin" }),
                inline_html: find_asset_html(assets, "comment-bubble-solar"),
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_templates_instantiate_cleanly_and_unknown_returns_none() {
        assert_eq!(builtin_templates().len(), 2);
        assert!(instantiate_template("non-existent", None, None, None, None, &[]).is_none());

        let short = instantiate_template(
            "talking-head-short",
            Some("My Short".to_string()),
            None,
            None,
            None,
            &[],
        )
        .expect("talking-head-short instantiates");
        assert_eq!(short.name, "My Short");
        assert_eq!(short.canvas.aspect_preset, AspectPreset::Vertical9x16);
        assert!(short.revision >= 4);
        assert!(
            short
                .tracks
                .iter()
                .find(|t| t.id == "t1")
                .unwrap()
                .clips
                .is_empty(),
            "templates must not pre-populate captions without narrated audio"
        );
        assert!(
            !short
                .tracks
                .iter()
                .find(|t| t.id == "g1")
                .unwrap()
                .clips
                .is_empty()
        );

        let explainer = instantiate_template(
            "faceless-explainer",
            None,
            None,
            None,
            Some(AspectPreset::Square1x1),
            &[],
        )
        .expect("faceless-explainer instantiates");
        assert_eq!(explainer.canvas.aspect_preset, AspectPreset::Square1x1);
        assert!(
            explainer
                .tracks
                .iter()
                .find(|t| t.id == "t1")
                .unwrap()
                .clips
                .is_empty(),
            "explainer template must not pre-populate captions without narrated audio"
        );
        assert_eq!(
            explainer
                .tracks
                .iter()
                .find(|t| t.id == "g1")
                .unwrap()
                .clips
                .len(),
            4
        );
    }
}
