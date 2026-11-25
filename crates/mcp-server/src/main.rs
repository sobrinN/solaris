use std::path::PathBuf;

use solaris_agent_runtime::{install_skills_to_dir, list_skills};
use solaris_mcp_server::run_stdio_http_bridge;
use tracing_subscriber::EnvFilter;

fn print_usage() {
    eprintln!(
        "Usage: solaris-mcp [serve | list-skills | install-skills [--target codex|antigravity|gemini|claude|cursor] [--path <dir>]]"
    );
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--help" | "-h" | "help") => {
            print_usage();
        }
        Some("list-skills") => {
            for s in list_skills() {
                println!("{}  ({}) — {}", s.name, s.uri, s.description);
            }
        }
        Some("install-skills") => {
            let target_dir = match resolve_install_dir(&args[1..]) {
                Ok(dir) => dir,
                Err(err) => {
                    eprintln!("Error: {err}");
                    print_usage();
                    std::process::exit(2);
                }
            };
            match install_skills_to_dir(&target_dir) {
                Ok(paths) => {
                    println!(
                        "Installed {} Solaris skill files across {} skills into {}:",
                        paths.len(),
                        list_skills().len(),
                        target_dir.display()
                    );
                    for p in paths {
                        println!("  - {}", p.display());
                    }
                }
                Err(err) => {
                    eprintln!("Failed to install skills: {err}");
                    std::process::exit(1);
                }
            }
        }
        None | Some("serve") => {
            let url = std::env::var("SOLARIS_SERVER_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:41731".to_string());
            let token = std::env::var("SOLARIS_AUTH_TOKEN")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            if let Err(err) = run_stdio_http_bridge(&url, token.as_deref()).await {
                eprintln!("solaris-mcp stdio error: {err}");
                std::process::exit(1);
            }
        }
        Some(other) => {
            eprintln!("Unknown subcommand '{other}'.");
            print_usage();
            std::process::exit(2);
        }
    }
}

fn resolve_install_dir(rest: &[String]) -> Result<PathBuf, String> {
    let mut target: Option<&str> = None;
    let mut custom_path: Option<PathBuf> = None;

    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--target" => {
                let Some(val) = rest.get(i + 1) else {
                    return Err(
                        "--target requires a value (codex|antigravity|gemini|claude|cursor)"
                            .to_string(),
                    );
                };
                target = Some(val.as_str());
                i += 2;
            }
            "--path" => {
                let Some(val) = rest.get(i + 1).filter(|s| !s.trim().is_empty()) else {
                    return Err("--path requires a non-empty directory path".to_string());
                };
                custom_path = Some(PathBuf::from(val));
                i += 2;
            }
            other => {
                return Err(format!("unknown argument '{other}'"));
            }
        }
    }

    if let Some(p) = custom_path {
        return Ok(p);
    }

    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .map_err(|_| "neither HOME nor USERPROFILE environment variable is set".to_string())?;

    match target.unwrap_or("codex") {
        "codex" => Ok(home.join(".codex").join("skills")),
        "antigravity" | "gemini" => Ok(home.join(".gemini").join("skills")),
        "claude" => Ok(home.join(".claude").join("skills")),
        "cursor" => Ok(home.join(".cursor").join("skills")),
        other => Err(format!(
            "unsupported --target '{other}'; expected codex, antigravity, gemini, claude, or cursor"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_skills_rejects_unknown_flags_and_targets() {
        assert!(resolve_install_dir(&["--unknown".to_string()]).is_err());
        assert!(resolve_install_dir(&["--target".to_string(), "invalid".to_string()]).is_err());
        assert!(resolve_install_dir(&["--target".to_string()]).is_err());
        assert_eq!(
            resolve_install_dir(&["--path".to_string(), "/tmp/my-skills".to_string()]).unwrap(),
            PathBuf::from("/tmp/my-skills")
        );
        if std::env::var("HOME").is_ok() || std::env::var("USERPROFILE").is_ok() {
            let cursor_dir =
                resolve_install_dir(&["--target".to_string(), "cursor".to_string()]).unwrap();
            assert!(cursor_dir.ends_with(PathBuf::from(".cursor").join("skills")));
        }
    }
}
