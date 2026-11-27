use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;
use solaris_core_ir::TranscriptWord;
use uuid::Uuid;

use crate::error::MediaError;
use crate::export::escape_ffmpeg_filter_path;
use crate::ffmpeg::MediaToolchain;

struct TempFileGuard(PathBuf);

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[derive(Debug, Deserialize)]
struct WhisperPayload {
    #[serde(default)]
    text: String,
    #[serde(default)]
    words: Vec<WhisperWordRaw>,
    #[serde(default)]
    segments: Vec<WhisperSegmentRaw>,
    #[serde(default)]
    transcription: Vec<WhisperCppItem>,
}

#[derive(Debug, Deserialize)]
struct WhisperWordRaw {
    #[serde(default)]
    word: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    start: Option<f64>,
    #[serde(default)]
    end: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct WhisperSegmentRaw {
    #[serde(default)]
    text: String,
    #[serde(default)]
    start: Option<f64>,
    #[serde(default)]
    end: Option<f64>,
    #[serde(default)]
    words: Vec<WhisperWordRaw>,
}

#[derive(Debug, Deserialize)]
struct WhisperCppItem {
    #[serde(default)]
    text: String,
    #[serde(default)]
    offsets: Option<WhisperCppOffsets>,
    #[serde(default)]
    tokens: Vec<WhisperCppToken>,
}

#[derive(Debug, Deserialize)]
struct WhisperCppToken {
    #[serde(default)]
    text: String,
    #[serde(default)]
    offsets: Option<WhisperCppOffsets>,
}

#[derive(Debug, Deserialize)]
struct WhisperCppOffsets {
    from: f64,
    to: f64,
}

/// FFmpeg 8 `af_whisper` `format=json` emits one JSON object per line with millisecond timestamps:
/// `{"start":0,"end":2500,"text":"Hello world"}`
#[derive(Debug, Deserialize)]
struct FfmpegWhisperNdjsonLine {
    #[serde(default)]
    start: Option<f64>,
    #[serde(default)]
    end: Option<f64>,
    #[serde(default)]
    text: String,
}

pub fn parse_whisper_json(raw: &str) -> Result<Vec<TranscriptWord>, MediaError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    // Try standard single-object JSON first (OpenAI verbose_json / whisper.cpp JSON)
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed)
        && let Some(obj) = val.as_object()
    {
        // Detect single-line FFmpeg af_whisper object: has start/end/text but no words/segments/transcription
        if !obj.contains_key("words")
            && !obj.contains_key("segments")
            && !obj.contains_key("transcription")
            && obj.contains_key("start")
            && obj.contains_key("end")
            && let Ok(line_item) = serde_json::from_value::<FfmpegWhisperNdjsonLine>(val.clone())
        {
            let mut out = Vec::new();
            if let (Some(start_ms), Some(end_ms)) = (line_item.start, line_item.end) {
                distribute_segment_words(
                    &mut out,
                    &line_item.text,
                    start_ms / 1000.0,
                    end_ms / 1000.0,
                );
            }
            return Ok(out);
        }

        let payload: WhisperPayload = serde_json::from_value(val)
            .map_err(|e| MediaError::ParseError(format!("invalid whisper JSON: {e}")))?;
        return extract_payload_words(payload);
    }

    // Fallback: newline-delimited JSONL emitted by FFmpeg 8 `af_whisper` (`format=json`)
    let mut out = Vec::new();
    let mut parsed_lines = 0usize;
    for line in trimmed.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        let item: FfmpegWhisperNdjsonLine = serde_json::from_str(l)
            .map_err(|e| MediaError::ParseError(format!("invalid whisper JSON/JSONL: {e}")))?;
        parsed_lines += 1;
        if let (Some(start_ms), Some(end_ms)) = (item.start, item.end) {
            let start_sec = start_ms / 1000.0;
            let end_sec = end_ms / 1000.0;
            distribute_segment_words(&mut out, &item.text, start_sec, end_sec);
        }
    }

    if parsed_lines == 0 {
        return Err(MediaError::ParseError(
            "empty whisper JSON payload".to_string(),
        ));
    }

    Ok(out)
}

fn extract_payload_words(payload: WhisperPayload) -> Result<Vec<TranscriptWord>, MediaError> {
    let mut out = Vec::new();

    if !payload.words.is_empty() {
        for w in payload.words {
            if let (Some(start), Some(end)) = (w.start, w.end) {
                push_word(&mut out, choose_word_text(&w.word, &w.text), start, end);
            }
        }
        if !out.is_empty() {
            return Ok(out);
        }
    }

    if !payload.segments.is_empty() {
        for seg in payload.segments {
            let before_len = out.len();
            if !seg.words.is_empty() {
                for w in seg.words {
                    if let (Some(start), Some(end)) = (w.start, w.end) {
                        push_word(&mut out, choose_word_text(&w.word, &w.text), start, end);
                    }
                }
            }
            if out.len() == before_len
                && let (Some(start), Some(end)) = (seg.start, seg.end)
            {
                distribute_segment_words(&mut out, &seg.text, start, end);
            }
        }
        if !out.is_empty() {
            return Ok(out);
        }
    }

    if !payload.transcription.is_empty() {
        for item in payload.transcription {
            if !item.tokens.is_empty() {
                let before_len = out.len();
                append_whisper_cpp_tokens(&mut out, &item.tokens);
                if out.len() > before_len {
                    continue;
                }
            }
            if let Some(off) = item.offsets {
                let start = off.from / 1000.0;
                let end = off.to / 1000.0;
                distribute_segment_words(&mut out, &item.text, start, end);
            }
        }
        return Ok(out);
    }

    if !payload.text.trim().is_empty() {
        return Err(MediaError::ParseError(
            "transcript JSON contains text without word or segment timestamps".to_string(),
        ));
    }

    Ok(out)
}

fn append_whisper_cpp_tokens(out: &mut Vec<TranscriptWord>, tokens: &[WhisperCppToken]) {
    let mut current_text = String::new();
    let mut current_start: Option<f64> = None;
    let mut current_end: f64 = 0.0;

    let flush =
        |out: &mut Vec<TranscriptWord>, text: &mut String, start: &mut Option<f64>, end: f64| {
            if let Some(st) = start.take() {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    push_word(out, trimmed, st, end);
                }
            }
            text.clear();
        };

    for tok in tokens {
        let raw = tok.text.as_str();
        let trimmed = raw.trim();
        if trimmed.is_empty()
            || (trimmed.starts_with('[') && trimmed.ends_with(']'))
            || (trimmed.starts_with("<|") && trimmed.ends_with("|>"))
        {
            continue;
        }
        let Some(off) = &tok.offsets else {
            continue;
        };
        let st = (off.from / 1000.0).max(0.0);
        let en = (off.to / 1000.0).max(st);
        let starts_new_word = raw.starts_with(char::is_whitespace) || current_start.is_none();
        if starts_new_word && current_start.is_some() {
            flush(out, &mut current_text, &mut current_start, current_end);
        }
        if current_start.is_none() {
            current_start = Some(st);
        }
        current_text.push_str(trimmed);
        current_end = en.max(current_end);
    }
    flush(out, &mut current_text, &mut current_start, current_end);
}

fn choose_word_text<'a>(a: &'a str, b: &'a str) -> &'a str {
    if !a.trim().is_empty() { a } else { b }
}

fn push_word(out: &mut Vec<TranscriptWord>, raw_text: &str, start: f64, end: f64) {
    let word = raw_text.trim();
    if word.is_empty() || !start.is_finite() || !end.is_finite() || start < 0.0 || end < start {
        return;
    }
    let normalized_end = if (end - start) < 1e-4 {
        start + 0.05
    } else {
        end
    };
    out.push(TranscriptWord {
        word: word.to_string(),
        start_sec: start,
        end_sec: normalized_end,
    });
}

fn distribute_segment_words(out: &mut Vec<TranscriptWord>, text: &str, start: f64, end: f64) {
    if !start.is_finite() || !end.is_finite() || start < 0.0 || end < start {
        return;
    }
    let tokens: Vec<&str> = text.split_whitespace().filter(|t| !t.is_empty()).collect();
    if tokens.is_empty() {
        return;
    }
    let effective_end = if (end - start) < 1e-4 {
        start + 0.05 * tokens.len() as f64
    } else {
        end
    };
    let total_chars: usize = tokens.iter().map(|t| t.chars().count().max(1)).sum();
    let span = effective_end - start;
    let mut cursor = start;
    for (idx, token) in tokens.iter().enumerate() {
        let weight = token.chars().count().max(1) as f64 / total_chars.max(1) as f64;
        let next = if idx + 1 == tokens.len() {
            effective_end
        } else {
            (cursor + span * weight).min(effective_end)
        };
        if next > cursor {
            out.push(TranscriptWord {
                word: (*token).to_string(),
                start_sec: cursor,
                end_sec: next,
            });
        }
        cursor = next;
    }
}

pub fn validate_language_code(lang: &str) -> Result<&str, MediaError> {
    let trimmed = lang.trim();
    if trimmed.is_empty()
        || trimmed.len() > 16
        || !trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(MediaError::ParseError(format!(
            "invalid language code '{lang}': expected 1..=16 ASCII [a-zA-Z0-9_-]"
        )));
    }
    Ok(trimmed)
}

pub fn discover_whisper_model(workspace_dir: Option<&Path>) -> Option<PathBuf> {
    if let Ok(env_path) = std::env::var("SOLARIS_WHISPER_MODEL") {
        let trimmed = env_path.trim();
        if !trimmed.is_empty() {
            let candidate = PathBuf::from(trimmed);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    let mut search_dirs: Vec<PathBuf> = Vec::new();
    if let Some(ws) = workspace_dir {
        search_dirs.push(ws.join("models"));
    }
    if let Ok(home) = std::env::var("HOME") {
        let h = PathBuf::from(home);
        search_dirs.push(h.join(".cache").join("whisper"));
        search_dirs.push(h.join(".local").join("share").join("whisper"));
        search_dirs.push(
            h.join("Library")
                .join("Application Support")
                .join("whisper"),
        );
    }
    search_dirs.push(PathBuf::from("/opt/homebrew/share/whisper-cpp"));
    search_dirs.push(PathBuf::from("/usr/local/share/whisper-cpp"));

    let mut best: Option<(usize, PathBuf)> = None;
    let priority = [
        "large-v3-turbo",
        "large-v3",
        "medium",
        "small",
        "base",
        "tiny",
    ];

    for dir in search_dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let lower = name.to_ascii_lowercase();
            if !lower.starts_with("ggml-")
                || !lower.ends_with(".bin")
                || lower.starts_with("for-tests-")
            {
                continue;
            }
            let rank = priority
                .iter()
                .position(|p| lower.contains(p))
                .unwrap_or(priority.len());
            if best.as_ref().is_none_or(|(best_rank, _)| rank < *best_rank) {
                best = Some((rank, path));
            }
        }
    }

    best.map(|(_, path)| path)
}

pub fn transcribe_via_whisper_cli(
    toolchain: &MediaToolchain,
    input: &Path,
    model_path: &Path,
    language: Option<&str>,
    work_dir: &Path,
) -> Result<Vec<TranscriptWord>, MediaError> {
    let ffmpeg = toolchain
        .ffmpeg_path
        .as_ref()
        .ok_or_else(|| MediaError::ToolchainUnavailable("ffmpeg not found".to_string()))?;
    let whisper_cli = toolchain
        .whisper_cli_path
        .as_ref()
        .ok_or_else(|| MediaError::ToolchainUnavailable("whisper-cli not found".to_string()))?;

    if !model_path.is_file() {
        return Err(MediaError::StorageError(format!(
            "whisper model file not found: {}",
            model_path.display()
        )));
    }

    let lang = match language.map(str::trim).filter(|s| !s.is_empty()) {
        Some(l) => validate_language_code(l)?,
        None => "auto",
    };

    if !work_dir.as_os_str().is_empty() {
        std::fs::create_dir_all(work_dir)?;
    }

    let job_id = Uuid::new_v4();
    let wav_path = work_dir.join(format!("whisper_in_{job_id}.wav"));
    let out_base = work_dir.join(format!("whisper_out_{job_id}"));
    let out_json = work_dir.join(format!("whisper_out_{job_id}.json"));
    let _wav_guard = TempFileGuard(wav_path.clone());
    let _json_guard = TempFileGuard(out_json.clone());

    let wav_out = Command::new(ffmpeg)
        .args(["-y", "-hide_banner", "-v", "error", "-vn", "-i"])
        .arg(input)
        .args(["-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
        .arg(&wav_path)
        .output()?;

    if !wav_out.status.success() {
        return Err(MediaError::ProcessFailed(
            String::from_utf8_lossy(&wav_out.stderr).to_string(),
        ));
    }

    let cli_out = Command::new(whisper_cli)
        .arg("-m")
        .arg(model_path)
        .arg("-f")
        .arg(&wav_path)
        .args(["-l", lang, "-ojf", "-of"])
        .arg(&out_base)
        .output()?;

    if !cli_out.status.success() {
        let stderr = String::from_utf8_lossy(&cli_out.stderr);
        let stdout = String::from_utf8_lossy(&cli_out.stdout);
        return Err(MediaError::ProcessFailed(if stderr.trim().is_empty() {
            stdout.to_string()
        } else {
            stderr.to_string()
        }));
    }

    let raw = std::fs::read_to_string(&out_json)?;
    parse_whisper_json(&raw)
}

pub fn transcribe_via_ffmpeg_whisper(
    toolchain: &MediaToolchain,
    path: &Path,
    model_path: &Path,
    language: Option<&str>,
    work_dir: &Path,
) -> Result<Vec<TranscriptWord>, MediaError> {
    let caps = toolchain.detect_capabilities();
    transcribe_via_ffmpeg_whisper_checked(
        toolchain,
        caps.has_whisper_filter,
        path,
        model_path,
        language,
        work_dir,
    )
}

pub fn transcribe_via_ffmpeg_whisper_checked(
    toolchain: &MediaToolchain,
    has_whisper_filter: bool,
    path: &Path,
    model_path: &Path,
    language: Option<&str>,
    work_dir: &Path,
) -> Result<Vec<TranscriptWord>, MediaError> {
    let ffmpeg = toolchain
        .ffmpeg_path
        .as_ref()
        .ok_or_else(|| MediaError::ToolchainUnavailable("ffmpeg not found".to_string()))?;

    if !has_whisper_filter {
        return Err(MediaError::ToolchainUnavailable(
            "ffmpeg whisper filter is not available in this build".to_string(),
        ));
    }
    if !model_path.is_file() {
        return Err(MediaError::StorageError(format!(
            "whisper model file not found: {}",
            model_path.display()
        )));
    }

    let lang = match language.map(str::trim).filter(|s| !s.is_empty()) {
        Some(l) => validate_language_code(l)?,
        None => "auto",
    };

    if !work_dir.as_os_str().is_empty() {
        std::fs::create_dir_all(work_dir)?;
    }
    let dest_json = work_dir.join(format!("whisper_{}.json", Uuid::new_v4()));
    let _dest_guard = TempFileGuard(dest_json.clone());

    let escaped_model = escape_ffmpeg_filter_path(&model_path.to_string_lossy());
    let escaped_dest = escape_ffmpeg_filter_path(&dest_json.to_string_lossy());

    let filter = format!(
        "whisper=model={escaped_model}:language={lang}:format=json:destination={escaped_dest}"
    );

    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-v", "error", "-vn", "-i"])
        .arg(path)
        .args(["-af", &filter, "-f", "null", "-"])
        .output()?;

    if !out.status.success() {
        return Err(MediaError::ProcessFailed(
            String::from_utf8_lossy(&out.stderr).to_string(),
        ));
    }

    let raw = std::fs::read_to_string(&dest_json)?;
    parse_whisper_json(&raw)
}

pub fn extract_audio_for_stt(
    toolchain: &MediaToolchain,
    input: &Path,
    output_mp3: &Path,
) -> Result<u64, MediaError> {
    let ffmpeg = toolchain
        .ffmpeg_path
        .as_ref()
        .ok_or_else(|| MediaError::ToolchainUnavailable("ffmpeg not found".to_string()))?;

    if let Some(parent) = output_mp3.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }

    let out = Command::new(ffmpeg)
        .args(["-y", "-hide_banner", "-v", "error", "-vn", "-i"])
        .arg(input)
        .args(["-ac", "1", "-ar", "16000", "-b:a", "64k"])
        .arg(output_mp3)
        .output()?;

    if !out.status.success() {
        return Err(MediaError::ProcessFailed(
            String::from_utf8_lossy(&out.stderr).to_string(),
        ));
    }

    let meta = std::fs::metadata(output_mp3)?;
    Ok(meta.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openrouter_verbose_json_words_with_zero_duration_and_null_tolerance() {
        let json = r#"{
            "text": "I use AI tools",
            "words": [
                {"word": "I", "start": 0.12, "end": 0.12},
                {"word": "bad", "start": null, "end": 0.40},
                {"word": " ", "start": 0.48, "end": 0.50},
                {"word": "AI", "start": 0.52, "end": 0.85},
                {"word": "tools", "start": 0.86, "end": 1.30}
            ]
        }"#;
        let words = parse_whisper_json(json).expect("parse words");
        assert_eq!(words.len(), 3);
        assert_eq!(words[0].word, "I");
        assert!((words[0].start_sec - 0.12).abs() < 1e-6);
        assert!((words[0].end_sec - 0.17).abs() < 1e-6);
        assert_eq!(words[2].word, "tools");
    }

    #[test]
    fn parses_ffmpeg8_af_whisper_ndjson_milliseconds() {
        let ndjson = "{\"start\":0,\"end\":1500,\"text\":\"Ship fast\"}\n{\"start\":1600,\"end\":3000,\"text\":\"with Solaris\"}\n";
        let words = parse_whisper_json(ndjson).expect("parse af_whisper ndjson");
        assert_eq!(words.len(), 4);
        assert_eq!(words[0].word, "Ship");
        assert!((words[0].start_sec - 0.0).abs() < 1e-6);
        assert_eq!(words[2].word, "with");
        assert!((words[2].start_sec - 1.6).abs() < 1e-6);
        assert!((words[3].end_sec - 3.0).abs() < 1e-6);
    }

    #[test]
    fn rejects_text_only_json_without_timestamps() {
        let text_only = r#"{"text": "Hello world without timestamps"}"#;
        assert!(parse_whisper_json(text_only).is_err());
    }

    #[test]
    fn escapes_ffmpeg_filter_path_two_level_unquoted() {
        let escaped = escape_ffmpeg_filter_path("/tmp/o'reilly/model:v1,a[0];b\\c.bin");
        assert_eq!(
            escaped,
            "/tmp/o\\\\\\'reilly/model\\\\:v1\\,a\\[0\\]\\;b\\\\\\\\c.bin"
        );
    }

    #[test]
    fn falls_back_to_segments_when_words_have_only_invalid_timestamps() {
        let json = r#"{
            "words": [
                {"word": "broken", "start": null, "end": null},
                {"word": "bad", "start": -1.0, "end": 0.5}
            ],
            "segments": [
                {"start": 0.5, "end": 2.5, "text": "clean fallback words"}
            ]
        }"#;
        let words = parse_whisper_json(json).expect("fallback to segments");
        assert_eq!(words.len(), 3);
        assert_eq!(words[0].word, "clean");
        assert_eq!(words[2].word, "words");
        assert!((words[0].start_sec - 0.5).abs() < 1e-6);
        assert!((words[2].end_sec - 2.5).abs() < 1e-6);
    }

    #[test]
    fn distributes_segment_only_whisper_json_proportionally() {
        let json = r#"{
            "segments": [
                {"start": 1.0, "end": 3.0, "text": "ship faster today"}
            ]
        }"#;
        let words = parse_whisper_json(json).expect("parse segments");
        assert_eq!(words.len(), 3);
        assert_eq!(words[0].word, "ship");
        assert!((words[0].start_sec - 1.0).abs() < 1e-6);
        assert!((words[2].end_sec - 3.0).abs() < 1e-6);
        assert!(words[1].start_sec > words[0].start_sec);
    }

    #[test]
    fn rejects_malformed_whisper_json() {
        assert!(parse_whisper_json("{not valid").is_err());
    }

    #[test]
    fn validates_language_codes_against_filter_injection() {
        assert_eq!(validate_language_code("en").unwrap(), "en");
        assert_eq!(validate_language_code("pt-BR").unwrap(), "pt-BR");
        assert_eq!(validate_language_code("auto").unwrap(), "auto");
        assert!(validate_language_code("en:a=b").is_err());
        assert!(validate_language_code("en,volume=0").is_err());
        assert!(validate_language_code("").is_err());
    }

    #[test]
    fn parses_whisper_cpp_ojf_tokens_merging_subwords_and_skipping_control_tokens() {
        let json = r#"{
            "transcription": [
                {
                    "offsets": { "from": 0, "to": 1400 },
                    "text": " Stand by me",
                    "tokens": [
                        { "text": "[_BEG_]", "offsets": { "from": 0, "to": 0 } },
                        { "text": " Stand", "offsets": { "from": 100, "to": 550 } },
                        { "text": "ing", "offsets": { "from": 550, "to": 800 } },
                        { "text": " by", "offsets": { "from": 820, "to": 1020 } },
                        { "text": " me", "offsets": { "from": 1040, "to": 1360 } },
                        { "text": "[_TT_68]", "offsets": { "from": 1360, "to": 1400 } }
                    ]
                }
            ]
        }"#;
        let words = parse_whisper_json(json).expect("parse whisper.cpp -ojf tokens");
        assert_eq!(words.len(), 3);
        assert_eq!(words[0].word, "Standing");
        assert!((words[0].start_sec - 0.1).abs() < 1e-6);
        assert!((words[0].end_sec - 0.8).abs() < 1e-6);
        assert_eq!(words[1].word, "by");
        assert_eq!(words[2].word, "me");
        assert!((words[2].end_sec - 1.36).abs() < 1e-6);
    }
}
