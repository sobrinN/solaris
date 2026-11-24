use std::path::{Path, PathBuf};

use include_dir::{Dir, include_dir};
use serde::Serialize;
use solaris_core_ir::AgentRole;

static SKILLS_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../skills");

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillDoc {
    pub name: String,
    pub description: String,
    pub uri: String,
    pub content: String,
}

fn parse_skill_markdown(fallback_name: &str, raw: &str) -> SkillDoc {
    let mut name = fallback_name.to_string();
    let mut description = String::new();

    let trimmed = raw.trim_start();
    if let Some(after_open) = trimmed.strip_prefix("---")
        && let Some(end_idx) = after_open.find("\n---")
    {
        let fm = &after_open[..end_idx];
        for line in fm.lines() {
            let line = line.trim();
            if let Some(val) = line.strip_prefix("name:") {
                let v = val.trim().trim_matches('"').trim_matches('\'');
                if !v.is_empty() {
                    name = v.to_string();
                }
            } else if let Some(val) = line.strip_prefix("description:") {
                let v = val.trim().trim_matches('"').trim_matches('\'');
                if !v.is_empty() {
                    description = v.to_string();
                }
            }
        }
    }

    SkillDoc {
        uri: format!("skill://{name}"),
        name,
        description,
        content: raw.to_string(),
    }
}

pub fn list_skills() -> Vec<SkillDoc> {
    let mut skills = Vec::new();
    for entry in SKILLS_DIR.dirs() {
        let dir_name = entry
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        if let Some(file) = entry.get_file(entry.path().join("SKILL.md"))
            && let Some(raw) = file.contents_utf8()
        {
            skills.push(parse_skill_markdown(dir_name, raw));
        }
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

pub fn get_skill(name_or_uri: &str) -> Option<SkillDoc> {
    let key = name_or_uri
        .trim()
        .strip_prefix("skill://")
        .unwrap_or(name_or_uri.trim());
    list_skills().into_iter().find(|s| s.name == key)
}

pub fn skills_for_role(role: AgentRole) -> Vec<SkillDoc> {
    let names: &[&str] = match role {
        AgentRole::Director => &["director-core", "talking-head-shorts", "faceless-explainer"],
        AgentRole::Razor => &["talking-head-shorts", "faceless-explainer"],
        AgentRole::BRoll => &["talking-head-shorts", "faceless-explainer"],
        AgentRole::Grade => &["color-and-look"],
        AgentRole::Caps => &["captions-and-gfx"],
        AgentRole::Gfx => &["captions-and-gfx"],
        AgentRole::Mix => &["audio-and-mix"],
        AgentRole::User | AgentRole::McpExternal => &["director-core"],
    };
    names.iter().filter_map(|n| get_skill(n)).collect()
}

pub fn install_skills_to_dir(target_dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(target_dir)?;
    let mut installed = Vec::new();
    for skill in list_skills() {
        let dir = target_dir.join(&skill.name);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("SKILL.md");
        std::fs::write(&path, skill.content)?;
        installed.push(path);
    }
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_all_six_core_skills() {
        let skills = list_skills();
        assert_eq!(skills.len(), 6);
        for required in [
            "director-core",
            "talking-head-shorts",
            "faceless-explainer",
            "color-and-look",
            "captions-and-gfx",
            "audio-and-mix",
        ] {
            let doc = get_skill(required).expect("missing skill");
            assert!(!doc.description.is_empty());
            assert!(!doc.content.is_empty());
            assert_eq!(doc.uri, format!("skill://{required}"));
        }
    }
}
