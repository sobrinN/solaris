use std::path::PathBuf;
use std::process::Command;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct MediaToolchain {
    pub ffmpeg_path: Option<PathBuf>,
    pub ffprobe_path: Option<PathBuf>,
    pub chromium_path: Option<PathBuf>,
    pub whisper_cli_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolchainCapabilities {
    pub ffmpeg_available: bool,
    pub ffprobe_available: bool,
    pub chromium_available: bool,
    pub ffmpeg_version: Option<String>,
    pub hw_encoders: Vec<String>,
    pub has_whisper_filter: bool,
    #[serde(default)]
    pub whisper_cli_available: bool,
    #[serde(default)]
    pub whisper_model_ready: bool,
}

impl MediaToolchain {
    pub fn from_env() -> Self {
        let ffmpeg_path = resolve_binary("SOLARIS_FFMPEG_PATH", "ffmpeg");
        let ffprobe_path = resolve_binary("SOLARIS_FFPROBE_PATH", "ffprobe");
        let chromium_path = detect_chromium_binary();
        let whisper_cli_path = resolve_whisper_cli_binary();
        Self {
            ffmpeg_path,
            ffprobe_path,
            chromium_path,
            whisper_cli_path,
        }
    }

    pub fn with_paths(ffmpeg: Option<PathBuf>, ffprobe: Option<PathBuf>) -> Self {
        Self {
            ffmpeg_path: ffmpeg,
            ffprobe_path: ffprobe,
            chromium_path: detect_chromium_binary(),
            whisper_cli_path: None,
        }
    }

    pub fn detect_capabilities(&self) -> ToolchainCapabilities {
        let mut ffmpeg_version = None;
        let mut ffmpeg_available = false;
        let mut hw_encoders = Vec::new();
        let mut has_whisper_filter = false;

        if let Some(ffmpeg) = &self.ffmpeg_path
            && let Ok(out) = Command::new(ffmpeg).arg("-version").output()
            && out.status.success()
        {
            ffmpeg_available = true;
            let text = String::from_utf8_lossy(&out.stdout);
            ffmpeg_version = text.lines().next().map(|l| l.trim().to_string());

            if let Ok(enc_out) = Command::new(ffmpeg)
                .args(["-hide_banner", "-encoders"])
                .output()
                && enc_out.status.success()
            {
                let enc_text = String::from_utf8_lossy(&enc_out.stdout);
                for candidate in [
                    "h264_videotoolbox",
                    "hevc_videotoolbox",
                    "h264_nvenc",
                    "hevc_nvenc",
                ] {
                    if enc_text.contains(candidate) {
                        hw_encoders.push(candidate.to_string());
                    }
                }
            }

            if let Ok(flt_out) = Command::new(ffmpeg)
                .args(["-hide_banner", "-filters"])
                .output()
                && flt_out.status.success()
            {
                let flt_text = String::from_utf8_lossy(&flt_out.stdout);
                has_whisper_filter = flt_text
                    .lines()
                    .any(|l| l.split_whitespace().nth(1) == Some("whisper"));
            }
        }

        let ffprobe_available = self
            .ffprobe_path
            .as_ref()
            .and_then(|p| Command::new(p).arg("-version").output().ok())
            .is_some_and(|o| o.status.success());

        let whisper_cli_available = self.whisper_cli_path.is_some();
        let whisper_model_ready = crate::transcribe::discover_whisper_model(None).is_some();

        ToolchainCapabilities {
            ffmpeg_available,
            ffprobe_available,
            chromium_available: self.chromium_path.is_some(),
            ffmpeg_version,
            hw_encoders,
            has_whisper_filter,
            whisper_cli_available,
            whisper_model_ready,
        }
    }
}

fn binary_candidates(base: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![format!("{base}.exe"), base.to_string()]
    } else {
        vec![base.to_string()]
    }
}

fn resolve_binary(env_key: &str, default_name: &str) -> Option<PathBuf> {
    if let Ok(val) = std::env::var(env_key) {
        let path = PathBuf::from(val.trim());
        if path.is_file() {
            return Some(path);
        }
    }
    let names = binary_candidates(default_name);
    if let Ok(path_env) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_env) {
            for name in &names {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    // Probe common macOS Homebrew locations when launched from GUI / Tauri
    for fallback_dir in ["/opt/homebrew/bin", "/usr/local/bin"] {
        for name in &names {
            let candidate = PathBuf::from(fallback_dir).join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

pub fn detect_chromium_binary() -> Option<PathBuf> {
    if let Ok(val) = std::env::var("SOLARIS_CHROME_PATH") {
        let path = PathBuf::from(val.trim());
        if path.is_file() {
            return Some(path);
        }
    }
    for app_path in [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files\Chromium\Application\chrome.exe",
    ] {
        let path = PathBuf::from(app_path);
        if path.is_file() {
            return Some(path);
        }
    }
    if let Ok(path_env) = std::env::var("PATH") {
        for base in [
            "chrome-headless-shell",
            "google-chrome",
            "chromium",
            "chromium-browser",
            "chrome",
        ] {
            let names = binary_candidates(base);
            for dir in std::env::split_paths(&path_env) {
                for name in &names {
                    let candidate = dir.join(name);
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
        }
    }
    None
}

fn resolve_whisper_cli_binary() -> Option<PathBuf> {
    if let Ok(val) = std::env::var("SOLARIS_WHISPER_PATH") {
        let path = PathBuf::from(val.trim());
        if path.is_file() {
            return Some(path);
        }
    }
    for name in ["whisper-cli", "whisper-cpp", "whisper"] {
        if let Some(found) = resolve_binary("__SOLARIS_NO_ENV__", name) {
            return Some(found);
        }
    }
    None
}
