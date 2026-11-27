use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use solaris_core_ir::Project;

use super::SwarmRunRequest;

pub const MAX_SELECTED_CLIPS: usize = 64;
pub const MAX_HISTORY_TURNS: usize = 12;
pub const MAX_HISTORY_CHARS: usize = 2000;
pub(crate) const NOTHING_TO_REFINE: &str = "Nothing to refine yet. Generate a cut first.";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SwarmRunMode {
    #[default]
    Auto,
    Generate,
    Refine,
}

impl SwarmRunMode {
    pub fn resolve(self, has_clips: bool) -> Self {
        match self {
            Self::Auto if has_clips => Self::Refine,
            Self::Auto => Self::Generate,
            explicit => explicit,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SwarmChatRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwarmChatTurn {
    pub role: SwarmChatRole,
    pub text: String,
}

pub(crate) fn timeline_has_clips(project: &Project) -> bool {
    project.tracks.iter().any(|t| !t.clips.is_empty())
}

pub fn prepare_swarm_request(req: &mut SwarmRunRequest, project: &Project) -> Result<(), String> {
    if req.selected_clip_ids.len() > MAX_SELECTED_CLIPS {
        return Err(format!(
            "selected_clip_ids accepts at most {MAX_SELECTED_CLIPS} clips"
        ));
    }
    let mut seen = HashSet::new();
    req.selected_clip_ids.retain(|id| seen.insert(id.clone()));

    let timeline_ids: HashSet<&str> = project
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter().map(|c| c.id.as_str()))
        .collect();
    if let Some(missing) = req
        .selected_clip_ids
        .iter()
        .find(|id| !timeline_ids.contains(id.as_str()))
    {
        return Err(format!("selected clip '{missing}' is not on the timeline"));
    }
    if let Some(playhead) = req.playhead_sec
        && !(playhead.is_finite() && playhead >= 0.0)
    {
        return Err("playhead_sec must be a finite number >= 0".to_string());
    }

    let has_clips = timeline_has_clips(project);
    if req.mode == SwarmRunMode::Refine && !has_clips {
        return Err(NOTHING_TO_REFINE.to_string());
    }
    req.mode = req.mode.resolve(has_clips);

    let keep_from = req
        .conversation_history
        .len()
        .saturating_sub(MAX_HISTORY_TURNS);
    req.conversation_history.drain(..keep_from);
    if req
        .conversation_history
        .iter()
        .any(|turn| turn.text.chars().count() > MAX_HISTORY_CHARS)
    {
        return Err(format!(
            "conversation_history text is limited to {MAX_HISTORY_CHARS} characters per turn"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use solaris_core_ir::{
        AspectPreset, AudioSettings, Clip, ClipKind, SwarmRunState, SwarmRunStatus, Transform2D,
    };

    use super::*;

    fn request(mode: SwarmRunMode) -> SwarmRunRequest {
        SwarmRunRequest {
            prompt: "Make the hook punchier".to_string(),
            model_override: None,
            allow_local_fallback: true,
            max_turns_per_worker: None,
            skip_qa: false,
            approved_plan: None,
            mode,
            selected_clip_ids: Vec::new(),
            playhead_sec: None,
            conversation_history: Vec::new(),
        }
    }

    fn project_with_clip(id: &str) -> Project {
        let mut project = Project::new_default("req", AspectPreset::Vertical9x16);
        let track = project
            .tracks
            .iter_mut()
            .find(|t| t.id == "v1")
            .expect("v1 track");
        track.clips.push(Clip {
            id: id.to_string(),
            label: id.to_string(),
            start_sec: 0.0,
            duration_sec: 2.0,
            source_in_sec: 0.0,
            transform: Transform2D::default(),
            audio: AudioSettings::default(),
            payload: ClipKind::Media {
                media_id: "m1".to_string(),
                ken_burns: false,
            },
        });
        project
    }

    #[test]
    fn mode_parses_lowercase_and_the_body_defaults_to_auto() {
        assert_eq!(
            serde_json::from_str::<SwarmRunMode>("\"refine\"").expect("refine"),
            SwarmRunMode::Refine
        );
        let minimal: SwarmRunRequest =
            serde_json::from_str(r#"{"prompt":"x"}"#).expect("minimal body");
        assert_eq!(minimal.mode, SwarmRunMode::Auto);
        assert!(minimal.allow_local_fallback);
        assert!(minimal.selected_clip_ids.is_empty());
        assert!(minimal.playhead_sec.is_none());
        assert!(minimal.conversation_history.is_empty());
    }

    #[test]
    fn chat_roles_accept_only_user_and_assistant() {
        assert!(serde_json::from_str::<SwarmChatRole>("\"system\"").is_err());
        assert_eq!(
            serde_json::from_str::<SwarmChatRole>("\"assistant\"").expect("assistant"),
            SwarmChatRole::Assistant
        );
    }

    #[test]
    fn auto_resolves_to_refine_only_when_the_timeline_has_clips() {
        assert_eq!(SwarmRunMode::Auto.resolve(false), SwarmRunMode::Generate);
        assert_eq!(SwarmRunMode::Auto.resolve(true), SwarmRunMode::Refine);
        assert_eq!(SwarmRunMode::Generate.resolve(true), SwarmRunMode::Generate);
    }

    #[test]
    fn refine_without_clips_is_rejected_and_auto_resolves_with_clips() {
        let empty = Project::new_default("empty", AspectPreset::Vertical9x16);
        let mut refine = request(SwarmRunMode::Refine);
        assert_eq!(
            prepare_swarm_request(&mut refine, &empty),
            Err(NOTHING_TO_REFINE.to_string())
        );

        let mut auto = request(SwarmRunMode::Auto);
        prepare_swarm_request(&mut auto, &project_with_clip("c1")).expect("valid");
        assert_eq!(auto.mode, SwarmRunMode::Refine);
    }

    #[test]
    fn selected_clips_must_exist_and_are_deduplicated() {
        let project = project_with_clip("c1");
        let mut known = request(SwarmRunMode::Auto);
        known.selected_clip_ids = vec!["c1".to_string(), "c1".to_string()];
        prepare_swarm_request(&mut known, &project).expect("valid");
        assert_eq!(known.selected_clip_ids, vec!["c1".to_string()]);

        let mut unknown = request(SwarmRunMode::Auto);
        unknown.selected_clip_ids = vec!["ghost".to_string()];
        assert!(
            prepare_swarm_request(&mut unknown, &project)
                .unwrap_err()
                .contains("ghost")
        );
    }

    #[test]
    fn selected_clip_list_is_capped_at_64() {
        let mut req = request(SwarmRunMode::Auto);
        req.selected_clip_ids = (0..=MAX_SELECTED_CLIPS).map(|i| format!("c{i}")).collect();
        assert!(prepare_swarm_request(&mut req, &project_with_clip("c1")).is_err());
    }

    #[test]
    fn playhead_must_be_finite_and_non_negative() {
        let project = project_with_clip("c1");
        for bad in [-0.1, f64::NAN, f64::INFINITY] {
            let mut req = request(SwarmRunMode::Auto);
            req.playhead_sec = Some(bad);
            assert!(prepare_swarm_request(&mut req, &project).is_err(), "{bad}");
        }
        let mut ok = request(SwarmRunMode::Auto);
        ok.playhead_sec = Some(3.2);
        assert!(prepare_swarm_request(&mut ok, &project).is_ok());
    }

    #[test]
    fn history_keeps_the_last_twelve_turns_and_rejects_overlong_text() {
        let project = project_with_clip("c1");
        let mut req = request(SwarmRunMode::Auto);
        req.conversation_history = (0..15)
            .map(|i| SwarmChatTurn {
                role: SwarmChatRole::User,
                text: format!("turn {i}"),
            })
            .collect();
        prepare_swarm_request(&mut req, &project).expect("valid");
        assert_eq!(req.conversation_history.len(), MAX_HISTORY_TURNS);
        assert_eq!(req.conversation_history[0].text, "turn 3");

        req.conversation_history = vec![SwarmChatTurn {
            role: SwarmChatRole::Assistant,
            text: "x".repeat(MAX_HISTORY_CHARS + 1),
        }];
        assert!(prepare_swarm_request(&mut req, &project).is_err());
    }

    #[test]
    fn status_serializes_refinement_summary_as_null_when_absent() {
        let now = Utc::now();
        let status = SwarmRunStatus {
            run_id: "run".to_string(),
            project_id: "project".to_string(),
            state: SwarmRunState::Queued,
            phase_label: "Queued".to_string(),
            progress: 0.0,
            model_used: None,
            used_local_fallback: false,
            active_roles: Vec::new(),
            ops_committed: 0,
            summary: None,
            error: None,
            qa_report: None,
            refinement_summary: None,
            started_at: now,
            updated_at: now,
        };
        let json = serde_json::to_value(&status).expect("serialize");
        assert!(json["refinement_summary"].is_null());
        assert_eq!(json["progress"], 0.0);
    }
}
