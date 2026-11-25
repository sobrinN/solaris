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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillResourceDoc {
    pub skill_name: String,
    pub relative_path: String,
    pub uri: String,
    pub description: String,
    pub mime_type: String,
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

fn first_heading_or_fallback(raw: &str, fallback: &str) -> String {
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(h) = trimmed.strip_prefix("# ") {
            let title = h.trim();
            if !title.is_empty() {
                return title.to_string();
            }
        }
    }
    fallback.to_string()
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

pub fn list_skill_resources() -> Vec<SkillResourceDoc> {
    let mut out = Vec::new();
    for entry in SKILLS_DIR.dirs() {
        let dir_name = entry
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        let skill_name = entry
            .get_file(entry.path().join("SKILL.md"))
            .and_then(|f| f.contents_utf8())
            .map(|raw| parse_skill_markdown(dir_name, raw).name)
            .unwrap_or_else(|| dir_name.to_string());

        let Some(refs_dir) = entry.get_dir(entry.path().join("references")) else {
            continue;
        };
        for file in refs_dir.files() {
            let Some(file_name) = file.path().file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !file_name.ends_with(".md") {
                continue;
            }
            let Some(raw) = file.contents_utf8() else {
                continue;
            };
            let rel = format!("references/{file_name}");
            let description = first_heading_or_fallback(raw, file_name);
            out.push(SkillResourceDoc {
                uri: format!("skill://{skill_name}/{rel}"),
                skill_name: skill_name.clone(),
                relative_path: rel,
                description,
                mime_type: "text/markdown".to_string(),
                content: raw.to_string(),
            });
        }
    }
    out.sort_by(|a, b| a.uri.cmp(&b.uri));
    out
}

pub fn get_skill_resource(uri_or_path: &str) -> Option<SkillResourceDoc> {
    let key = uri_or_path.trim();
    let normalized = if key.starts_with("skill://") {
        key.to_string()
    } else {
        format!("skill://{key}")
    };

    if let Some(skill) = get_skill(&normalized) {
        return Some(SkillResourceDoc {
            skill_name: skill.name,
            relative_path: "SKILL.md".to_string(),
            uri: skill.uri,
            description: skill.description,
            mime_type: "text/markdown".to_string(),
            content: skill.content,
        });
    }

    list_skill_resources()
        .into_iter()
        .find(|r| r.uri == normalized)
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

/// Returns progressive-disclosure reference docs needed directly by internal
/// Rust swarm roles (which cannot call MCP `resources/read`).
pub fn skill_references_for_role(role: AgentRole) -> Vec<SkillResourceDoc> {
    let uris: &[&str] = match role {
        AgentRole::Director => &["skill://director-core/references/direction-and-pacing.md"],
        AgentRole::Gfx => &[
            "skill://captions-and-gfx/references/gfx-contract.md",
            "skill://captions-and-gfx/references/starter-assets.md",
        ],
        _ => &[],
    };
    uris.iter().filter_map(|u| get_skill_resource(u)).collect()
}

fn write_dir_recursive(
    dir: &Dir<'_>,
    root_prefix: &Path,
    dest_root: &Path,
    installed: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    for file in dir.files() {
        let rel = file.path().strip_prefix(root_prefix).unwrap_or(file.path());
        let out_path = dest_root.join(rel);
        if let Some(parent) = out_path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&out_path, file.contents())?;
        installed.push(out_path);
    }
    for subdir in dir.dirs() {
        write_dir_recursive(subdir, root_prefix, dest_root, installed)?;
    }
    Ok(())
}

pub fn install_skills_to_dir(target_dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(target_dir)?;
    let mut installed = Vec::new();
    let mut entries: Vec<&Dir<'_>> = SKILLS_DIR.dirs().collect();
    entries.sort_by_key(|d| d.path());

    for entry in entries {
        let dir_name = entry
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        let Some(skill_file) = entry.get_file(entry.path().join("SKILL.md")) else {
            continue;
        };
        let Some(raw) = skill_file.contents_utf8() else {
            continue;
        };
        let parsed = parse_skill_markdown(dir_name, raw);
        let dest_root = target_dir.join(&parsed.name);
        std::fs::create_dir_all(&dest_root)?;
        write_dir_recursive(entry, entry.path(), &dest_root, &mut installed)?;
    }
    installed.sort();
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

    #[test]
    fn embeds_progressive_disclosure_skill_references() {
        let refs = list_skill_resources();
        assert!(
            refs.len() >= 6,
            "expected at least 6 reference docs, found {}",
            refs.len()
        );
        for uri in [
            "skill://director-core/references/tool-reference.md",
            "skill://director-core/references/direction-and-pacing.md",
            "skill://director-core/references/critique-and-qa.md",
            "skill://director-core/references/failure-recovery.md",
            "skill://captions-and-gfx/references/gfx-contract.md",
            "skill://captions-and-gfx/references/starter-assets.md",
        ] {
            let res = get_skill_resource(uri).expect("missing skill reference resource");
            assert_eq!(res.uri, uri);
            assert!(!res.content.is_empty());
        }
        let root = get_skill_resource("skill://director-core").expect("root skill lookup");
        assert_eq!(root.relative_path, "SKILL.md");

        let gfx_refs = skill_references_for_role(AgentRole::Gfx);
        assert!(gfx_refs.iter().any(|r| r.uri.ends_with("gfx-contract.md")));
        assert!(gfx_refs.iter().any(|r| r.uri.ends_with("starter-assets.md")));
        let dir_refs = skill_references_for_role(AgentRole::Director);
        assert!(dir_refs.iter().any(|r| r.uri.ends_with("direction-and-pacing.md")));
    }

    #[test]
    fn installs_skill_directories_with_references_and_metadata() {
        let tmp = std::env::temp_dir().join(format!(
            "solaris-skill-install-test-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = install_skills_to_dir(&tmp).expect("install skills");
        assert!(paths.iter().any(|p| p.ends_with("director-core/SKILL.md")));
        assert!(
            paths
                .iter()
                .any(|p| p.ends_with("director-core/references/tool-reference.md"))
        );
        assert!(
            paths
                .iter()
                .any(|p| p.ends_with("captions-and-gfx/references/gfx-contract.md"))
        );
        assert!(
            paths
                .iter()
                .any(|p| p.ends_with("director-core/agents/openai.yaml"))
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
