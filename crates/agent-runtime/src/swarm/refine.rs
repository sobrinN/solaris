use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use serde_json::{Value, json};
use solaris_core_ir::{AgentRole, Clip, ClipKind, Project};

use super::request::{MAX_HISTORY_TURNS, SwarmChatTurn};

pub(crate) const MAX_SNAPSHOT_CLIPS: usize = 60;
pub(crate) const MAX_REFINEMENT_ROLES: usize = 4;
const MAX_SUMMARY_CHARS: usize = 280;
const PREVIEW_WORDS: usize = 8;
const REFINABLE_ROLES: [AgentRole; 6] = [
    AgentRole::Razor,
    AgentRole::BRoll,
    AgentRole::Grade,
    AgentRole::Caps,
    AgentRole::Gfx,
    AgentRole::Mix,
];

pub(crate) const REFINEMENT_SYSTEM_PROMPT: &str = "You are Solaris Director in REFINEMENT mode. The user already has a finished cut and wants a targeted change. Read `request` and `cut` (tracks, clip ids, selected clips, playhead, recent conversation) and pick the smallest set of specialists that can make the change. Output ONLY one compact raw JSON object (no markdown, no commentary) with keys: \"summary\" (string, max 280 characters: what will change, or why nothing needs to change), \"target_roles\" (array, at most 4, from: razor = V1/A1 spine clips; b_roll = V2 inserts; grade = FX master grade; caps = T1 captions; gfx = G1 motion graphics; mix = A2 music and audio levels) and \"role_instructions\" (object mapping each target role to one concrete instruction that names the clip ids to edit). Rules: edit existing clips by id only; never ask for new scenes, duplicate clips or re-placed GFX; never target a role whose tracks are locked; return an empty target_roles array when nothing needs to change.";

pub(crate) struct RefineFocus<'a> {
    pub selected_clip_ids: &'a [String],
    pub playhead_sec: Option<f64>,
    pub history: &'a [SwarmChatTurn],
}

pub(crate) fn build_refine_snapshot(project: &Project, focus: &RefineFocus<'_>) -> Value {
    let selected: HashSet<&str> = focus.selected_clip_ids.iter().map(String::as_str).collect();
    let media_names: HashMap<&str, &str> = project
        .media_pool
        .iter()
        .map(|m| (m.id.as_str(), m.name.as_str()))
        .collect();

    let mut refs: Vec<(usize, usize)> = project
        .tracks
        .iter()
        .enumerate()
        .flat_map(|(ti, track)| (0..track.clips.len()).map(move |ci| (ti, ci)))
        .collect();
    let total = refs.len();
    refs.sort_by_key(|&(ti, ci)| !selected.contains(project.tracks[ti].clips[ci].id.as_str()));
    refs.truncate(MAX_SNAPSHOT_CLIPS);
    refs.sort_unstable();

    let tracks: Vec<Value> = project
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, track)| !track.clips.is_empty())
        .map(|(ti, track)| {
            let clips: Vec<Value> = refs
                .iter()
                .filter(|(t, _)| *t == ti)
                .map(|&(_, ci)| clip_summary(&track.clips[ci], &selected, &media_names))
                .collect();
            json!({ "id": track.id, "code": track.code, "locked": track.locked, "clips": clips })
        })
        .collect();

    let history: Vec<Value> = focus
        .history
        .iter()
        .skip(focus.history.len().saturating_sub(MAX_HISTORY_TURNS))
        .map(|turn| json!({ "role": turn.role, "text": turn.text }))
        .collect();

    let omitted = total - refs.len();
    json!({
        "canvas": {
            "width": project.canvas.width,
            "height": project.canvas.height,
            "aspect": project.canvas.aspect_preset,
        },
        "duration_sec": project.duration_sec(),
        "playhead_sec": focus.playhead_sec,
        "selected_clip_ids": focus.selected_clip_ids,
        "tracks": tracks,
        "omitted_clips": omitted,
        "truncation_note": (omitted > 0).then(|| format!(
            "{omitted} more clips are not listed; the selected clips are always included."
        )),
        "conversation": history,
    })
}

fn clip_summary(clip: &Clip, selected: &HashSet<&str>, media_names: &HashMap<&str, &str>) -> Value {
    let mut entry = json!({
        "id": clip.id,
        "label": clip.label,
        "start_sec": clip.start_sec,
        "duration_sec": clip.duration_sec,
        "selected": selected.contains(clip.id.as_str()),
    });
    let details = match &clip.payload {
        ClipKind::Media { media_id, .. } => json!({
            "kind": "media",
            "source": media_names.get(media_id.as_str()).copied().unwrap_or_default(),
        }),
        ClipKind::Caption { caption } => {
            let words: Vec<&str> = caption.words.iter().map(|w| w.text.as_str()).collect();
            json!({
                "kind": "caption",
                "words": words.len(),
                "preview": words.iter().take(PREVIEW_WORDS).copied().collect::<Vec<_>>().join(" "),
            })
        }
        ClipKind::Gfx { gfx } => json!({ "kind": "gfx", "asset_id": gfx.asset_id }),
        ClipKind::Grade { .. } => json!({ "kind": "grade" }),
    };
    if let (Value::Object(base), Value::Object(extra)) = (&mut entry, details) {
        base.extend(extra);
    }
    entry
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RefinementTarget {
    pub role: AgentRole,
    pub instruction: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RefinementPlan {
    pub summary: String,
    pub targets: Vec<RefinementTarget>,
}

#[derive(Deserialize)]
struct RawRefinementPlan {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    target_roles: Vec<String>,
    #[serde(default)]
    role_instructions: HashMap<String, String>,
}

pub(crate) fn parse_refinement_plan(raw: &str) -> Result<RefinementPlan, serde_json::Error> {
    let parsed: RawRefinementPlan = serde_json::from_str(raw)?;

    let mut roles: Vec<(AgentRole, String)> = Vec::new();
    for name in &parsed.target_roles {
        let key = normalize_role_name(name);
        let Some(role) = parse_refinable_role(&key) else {
            continue;
        };
        if roles.iter().any(|(existing, _)| *existing == role) {
            continue;
        }
        if roles.len() == MAX_REFINEMENT_ROLES {
            break;
        }
        roles.push((role, key));
    }

    let summary = match parsed.summary.trim() {
        "" if roles.is_empty() => "nothing in the cut needs changing".to_string(),
        "" => "Applied the requested change.".to_string(),
        text => text.chars().take(MAX_SUMMARY_CHARS).collect(),
    };

    let mut instructions: HashMap<String, String> = parsed
        .role_instructions
        .into_iter()
        .map(|(role, text)| (normalize_role_name(&role), text.trim().to_string()))
        .collect();
    let targets = roles
        .into_iter()
        .map(|(role, key)| {
            let instruction = match instructions.remove(&key) {
                Some(text) if !text.is_empty() => text,
                _ => summary.clone(),
            };
            RefinementTarget { role, instruction }
        })
        .collect();

    Ok(RefinementPlan { summary, targets })
}

fn normalize_role_name(name: &str) -> String {
    match name.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "broll" => "b_roll".to_string(),
        other => other.to_string(),
    }
}

fn parse_refinable_role(key: &str) -> Option<AgentRole> {
    let role: AgentRole = serde_json::from_value(Value::String(key.to_string())).ok()?;
    REFINABLE_ROLES.contains(&role).then_some(role)
}

pub(crate) fn no_change_summary(summary: &str) -> String {
    if summary.to_ascii_lowercase().starts_with("no change needed") {
        summary.to_string()
    } else {
        format!("No change needed: {summary}")
    }
}

pub(crate) fn refinement_tracks(role: AgentRole) -> &'static [&'static str] {
    match role {
        AgentRole::Razor => &["v1", "a1"],
        AgentRole::BRoll => &["v2"],
        AgentRole::Grade => &["fx"],
        AgentRole::Caps => &["t1"],
        AgentRole::Gfx => &["g1"],
        AgentRole::Mix => &["a2"],
        AgentRole::User | AgentRole::Director | AgentRole::McpExternal => &[],
    }
}

pub(crate) fn refinement_agent_id(role: AgentRole) -> &'static str {
    match role {
        AgentRole::Razor => "razor-01",
        AgentRole::BRoll => "broll-01",
        AgentRole::Grade => "grade-01",
        AgentRole::Caps => "caps-01",
        AgentRole::Gfx => "gfx-01",
        AgentRole::Mix => "mix-01",
        AgentRole::User | AgentRole::Director | AgentRole::McpExternal => "refine-01",
    }
}

#[cfg(test)]
mod tests;
