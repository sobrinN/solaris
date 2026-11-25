use solaris_core_ir::{
    AspectPreset, AudioSettings, CaptionBlock, CaptionPreset, CaptionWord, GfxInstance, MediaAsset,
    MediaIntelligenceManifest, MediaKind, Transform2D,
};

use super::*;
use crate::swarm::request::SwarmChatRole;
use crate::tools::tools_for_refinement;

fn push_clip(project: &mut Project, track_id: &str, id: &str, payload: ClipKind) {
    let track = project
        .tracks
        .iter_mut()
        .find(|t| t.id == track_id)
        .expect("track exists");
    track.clips.push(Clip {
        id: id.to_string(),
        label: format!("{id} label"),
        start_sec: 0.0,
        duration_sec: 3.0,
        source_in_sec: 0.0,
        transform: Transform2D::default(),
        audio: AudioSettings::default(),
        payload,
    });
}

fn cut_project() -> Project {
    let mut project = Project::new_default("snapshot", AspectPreset::Vertical9x16);
    project.media_pool.push(MediaAsset {
        id: "m1".to_string(),
        name: "talk.mp4".to_string(),
        kind: MediaKind::Video,
        role_hint: "a_roll".to_string(),
        path: "/media/talk.mp4".to_string(),
        added_at: chrono::Utc::now(),
        intelligence: MediaIntelligenceManifest::default(),
    });
    push_clip(
        &mut project,
        "v1",
        "hook-1",
        ClipKind::Media {
            media_id: "m1".to_string(),
            ken_burns: false,
        },
    );
    let words = (1..=10)
        .map(|i| CaptionWord {
            text: format!("w{i}"),
            start_sec: i as f64 * 0.2,
            end_sec: i as f64 * 0.2 + 0.15,
        })
        .collect();
    push_clip(
        &mut project,
        "t1",
        "cap-1",
        ClipKind::Caption {
            caption: CaptionBlock {
                words,
                preset: CaptionPreset::NeonGlow,
                font_family: "Sora".to_string(),
                highlight_color: "#F7B733".to_string(),
            },
        },
    );
    push_clip(
        &mut project,
        "g1",
        "card-1",
        ClipKind::Gfx {
            gfx: GfxInstance {
                asset_id: Some("stat-card".to_string()),
                props: json!({}),
                inline_html: None,
            },
        },
    );
    project
}

fn focus<'a>(selected: &'a [String], history: &'a [SwarmChatTurn]) -> RefineFocus<'a> {
    RefineFocus {
        selected_clip_ids: selected,
        playhead_sec: Some(3.2),
        history,
    }
}

fn clip_json<'a>(snapshot: &'a Value, clip_id: &str) -> &'a Value {
    snapshot["tracks"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|track| track["clips"].as_array().into_iter().flatten())
        .find(|clip| clip["id"] == clip_id)
        .expect("clip present in snapshot")
}

#[test]
fn snapshot_flags_selected_clips_and_names_their_media() {
    let project = cut_project();
    let selected = vec!["hook-1".to_string()];
    let snapshot = build_refine_snapshot(&project, &focus(&selected, &[]));
    assert_eq!(clip_json(&snapshot, "hook-1")["selected"], true);
    assert_eq!(clip_json(&snapshot, "cap-1")["selected"], false);
    assert_eq!(clip_json(&snapshot, "hook-1")["source"], "talk.mp4");
    assert_eq!(snapshot["playhead_sec"], 3.2);
}

#[test]
fn snapshot_summarizes_captions_by_word_count_and_first_eight_words() {
    let snapshot = build_refine_snapshot(&cut_project(), &focus(&[], &[]));
    let caption = clip_json(&snapshot, "cap-1");
    assert_eq!(caption["kind"], "caption");
    assert_eq!(caption["words"], 10);
    assert_eq!(caption["preview"], "w1 w2 w3 w4 w5 w6 w7 w8");
}

#[test]
fn snapshot_describes_gfx_clips_by_asset_and_omits_empty_tracks() {
    let snapshot = build_refine_snapshot(&cut_project(), &focus(&[], &[]));
    let gfx = clip_json(&snapshot, "card-1");
    assert_eq!(gfx["kind"], "gfx");
    assert_eq!(gfx["asset_id"], "stat-card");
    let track_ids: Vec<&str> = snapshot["tracks"]
        .as_array()
        .expect("tracks array")
        .iter()
        .filter_map(|t| t["id"].as_str())
        .collect();
    assert_eq!(track_ids, vec!["t1", "g1", "v1"]);
}

#[test]
fn snapshot_bounds_clip_count_and_always_keeps_selected_clips() {
    let mut project = Project::new_default("bounded", AspectPreset::Vertical9x16);
    for i in 0..100 {
        push_clip(
            &mut project,
            "v1",
            &format!("c{i}"),
            ClipKind::Grade {
                grade: Default::default(),
            },
        );
    }
    let selected = vec!["c99".to_string()];
    let snapshot = build_refine_snapshot(&project, &focus(&selected, &[]));
    let listed: usize = snapshot["tracks"]
        .as_array()
        .expect("tracks")
        .iter()
        .map(|t| t["clips"].as_array().map_or(0, Vec::len))
        .sum();
    assert_eq!(listed, MAX_SNAPSHOT_CLIPS);
    assert_eq!(snapshot["omitted_clips"], 40);
    assert!(
        snapshot["truncation_note"]
            .as_str()
            .is_some_and(|note| note.starts_with("40 more clips"))
    );
    assert_eq!(clip_json(&snapshot, "c99")["selected"], true);
}

#[test]
fn snapshot_keeps_only_the_last_twelve_conversation_turns() {
    let history: Vec<SwarmChatTurn> = (0..15)
        .map(|i| SwarmChatTurn {
            role: SwarmChatRole::User,
            text: format!("turn {i}"),
        })
        .collect();
    let snapshot = build_refine_snapshot(&cut_project(), &focus(&[], &history));
    let conversation = snapshot["conversation"].as_array().expect("conversation");
    assert_eq!(conversation.len(), MAX_HISTORY_TURNS);
    assert_eq!(conversation[0]["text"], "turn 3");
}

#[test]
fn plan_parsing_drops_unknown_and_disallowed_roles() {
    let plan = parse_refinement_plan(
        r#"{"summary":"Punch up the card","target_roles":["gfx","wizard","director","user"],"role_instructions":{"gfx":"Shrink the stat card on g1"}}"#,
    )
    .expect("valid plan");
    assert_eq!(plan.targets.len(), 1);
    assert_eq!(plan.targets[0].role, AgentRole::Gfx);
    assert_eq!(plan.targets[0].instruction, "Shrink the stat card on g1");
}

#[test]
fn plan_parsing_removes_duplicate_roles_case_insensitively() {
    let plan = parse_refinement_plan(r#"{"summary":"s","target_roles":["Gfx","gfx","GFX"]}"#)
        .expect("valid plan");
    assert_eq!(plan.targets.len(), 1);
}

#[test]
fn plan_parsing_caps_targets_at_four_roles() {
    let plan = parse_refinement_plan(
        r#"{"summary":"s","target_roles":["razor","b_roll","grade","caps","gfx","mix"]}"#,
    )
    .expect("valid plan");
    let roles: Vec<AgentRole> = plan.targets.iter().map(|t| t.role).collect();
    assert_eq!(
        roles,
        vec![
            AgentRole::Razor,
            AgentRole::BRoll,
            AgentRole::Grade,
            AgentRole::Caps
        ]
    );
}

#[test]
fn plan_parsing_falls_back_to_the_summary_when_an_instruction_is_missing() {
    let plan = parse_refinement_plan(
        r#"{"summary":"Shorten the captions","target_roles":["caps","broll"],"role_instructions":{"caps":"  Trim each caption to 4 words. "}}"#,
    )
    .expect("valid plan");
    assert_eq!(plan.targets[0].instruction, "Trim each caption to 4 words.");
    assert_eq!(plan.targets[1].role, AgentRole::BRoll);
    assert_eq!(plan.targets[1].instruction, "Shorten the captions");
}

#[test]
fn empty_targets_produce_a_no_op_plan_with_a_no_change_summary() {
    let plan = parse_refinement_plan(r#"{"summary":"The hook already works.","target_roles":[]}"#)
        .expect("valid plan");
    assert!(plan.targets.is_empty());
    assert_eq!(
        no_change_summary(&plan.summary),
        "No change needed: The hook already works."
    );
    assert_eq!(
        no_change_summary("No change needed: pacing is fine."),
        "No change needed: pacing is fine."
    );
}

#[test]
fn malformed_plan_json_is_a_parse_error() {
    assert!(parse_refinement_plan("not json").is_err());
    assert!(parse_refinement_plan(r#"{"target_roles":"gfx"}"#).is_err());
}

#[test]
fn plan_summary_is_capped_at_280_characters() {
    let long = "a".repeat(400);
    let plan = parse_refinement_plan(&format!(
        r#"{{"summary":"{long}","target_roles":["grade"]}}"#
    ))
    .expect("valid plan");
    assert_eq!(plan.summary.chars().count(), MAX_SUMMARY_CHARS);
}

#[test]
fn refinement_tools_keep_the_edit_tools_for_each_target_role() {
    let expected: [(AgentRole, &[&str]); 6] = [
        (
            AgentRole::Razor,
            &[
                "timeline_trim_clip",
                "timeline_delete_clip",
                "timeline_set_transform",
            ],
        ),
        (
            AgentRole::BRoll,
            &[
                "timeline_trim_clip",
                "timeline_delete_clip",
                "timeline_set_transform",
            ],
        ),
        (AgentRole::Grade, &["color_set_grade"]),
        (
            AgentRole::Caps,
            &[
                "captions_update_clip",
                "timeline_trim_clip",
                "timeline_delete_clip",
                "timeline_set_transform",
            ],
        ),
        (
            AgentRole::Gfx,
            &[
                "gfx_update_clip",
                "timeline_trim_clip",
                "timeline_delete_clip",
                "timeline_set_transform",
            ],
        ),
        (AgentRole::Mix, &["audio_set_mix"]),
    ];
    for (role, names) in expected {
        let offered: Vec<String> = tools_for_refinement(role)
            .into_iter()
            .map(|def| def.function.name)
            .collect();
        for name in names {
            assert!(offered.iter().any(|o| o == name), "{role:?} missing {name}");
        }
    }
}

#[test]
fn refinement_tools_never_offer_placement_or_asset_tools() {
    let gfx: Vec<String> = tools_for_refinement(AgentRole::Gfx)
        .into_iter()
        .map(|def| def.function.name)
        .collect();
    assert!(!gfx.iter().any(|n| n == "gfx_place" || n == "assets_save"));
    assert!(!gfx.iter().any(|n| n == "captions_update_clip"));
    let caps: Vec<String> = tools_for_refinement(AgentRole::Caps)
        .into_iter()
        .map(|def| def.function.name)
        .collect();
    assert!(
        !caps
            .iter()
            .any(|n| n == "captions_generate_from_transcript")
    );
}
