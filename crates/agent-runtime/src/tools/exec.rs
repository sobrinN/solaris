use std::collections::HashSet;

use serde_json::{Value, json};
use solaris_core_ir::{
    AspectPreset, AudioSettings, CaptionBlock, CaptionPreset, CaptionWord, Clip, ClipKind,
    ColorGrade, CommandMeta, ExportFormat, ExportQuality, ExportRequest, GfxInstance, MediaAsset,
    MediaKind, Project, RgbTriplet, SavedCodeAsset, TimelineCommand, TrackKind, TranscriptWord,
    Transform2D, validate_single_asset,
};

use crate::backend::SolarisBackend;
use crate::error::AgentError;
use crate::lint::lint_gfx_html;
use crate::swarm::run_qa_pass;
use crate::tools::schemas::{ToolOutput, normalize_tool_name};

fn deep_merge_json(base: &Value, patch: &Value) -> Value {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            let mut merged = b.clone();
            for (k, v) in p {
                let next = match merged.get(k) {
                    Some(existing) => deep_merge_json(existing, v),
                    None => v.clone(),
                };
                merged.insert(k.clone(), next);
            }
            Value::Object(merged)
        }
        (_, other) => other.clone(),
    }
}

fn parse_optional_enum<T: serde::de::DeserializeOwned>(
    args: &Value,
    key: &str,
) -> Result<Option<T>, AgentError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value::<T>(v.clone())
            .map(Some)
            .map_err(|e| AgentError::ToolError(format!("invalid argument '{key}': {e}"))),
    }
}

fn req_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, AgentError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| AgentError::ToolError(format!("missing required string argument '{key}'")))
}

fn extract_inline_html(args: &Value) -> Option<String> {
    ["inline_html", "html_source", "html"]
        .into_iter()
        .find_map(|key| {
            args.get(key)
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
}

fn req_f64(args: &Value, key: &str) -> Result<f64, AgentError> {
    args.get(key)
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite())
        .ok_or_else(|| AgentError::ToolError(format!("missing required number argument '{key}'")))
}

fn validate_safe_id(id: &str) -> Result<(), AgentError> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(AgentError::ToolError(format!(
            "invalid identifier '{id}': expected 1..=64 ASCII [a-zA-Z0-9_-]"
        )));
    }
    Ok(())
}

/// Rewrites caption text while keeping it in sync with narration: when the word
/// count is unchanged the original transcript timings are preserved 1:1; otherwise
/// words are redistributed across the original spoken span.
pub(crate) fn retime_caption_words(
    original: &[CaptionWord],
    text: &str,
    clip: &Clip,
) -> Vec<CaptionWord> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.len() == original.len() {
        return original
            .iter()
            .zip(tokens)
            .map(|(w, t)| CaptionWord {
                text: t.to_string(),
                start_sec: w.start_sec,
                end_sec: w.end_sec,
            })
            .collect();
    }
    let span_start = original
        .first()
        .map(|w| w.start_sec)
        .unwrap_or(clip.start_sec);
    let span_end = original
        .last()
        .map(|w| w.end_sec)
        .unwrap_or(clip.start_sec + clip.duration_sec)
        .max(span_start + 0.05);
    let step = (span_end - span_start) / tokens.len().max(1) as f64;
    tokens
        .iter()
        .enumerate()
        .map(|(i, t)| CaptionWord {
            text: (*t).to_string(),
            start_sec: span_start + step * i as f64,
            end_sec: span_start + step * (i + 1) as f64,
        })
        .collect()
}

fn apply_rgb_patch(target: &mut RgbTriplet, obj: Option<&Value>) {
    let Some(v) = obj.and_then(|v| v.as_object()) else {
        return;
    };
    if let Some(r) = v
        .get("r")
        .and_then(|n| n.as_f64())
        .filter(|n| n.is_finite())
    {
        target.r = r;
    }
    if let Some(g) = v
        .get("g")
        .and_then(|n| n.as_f64())
        .filter(|n| n.is_finite())
    {
        target.g = g;
    }
    if let Some(b) = v
        .get("b")
        .and_then(|n| n.as_f64())
        .filter(|n| n.is_finite())
    {
        target.b = b;
    }
}

fn map_transcript_to_timeline(
    words: &[TranscriptWord],
    media_id: &str,
    speech_clips: &[&Clip],
) -> Vec<TranscriptWord> {
    let mut matching: Vec<&Clip> = speech_clips
        .iter()
        .copied()
        .filter(|c| matches!(&c.payload, ClipKind::Media { media_id: mid, .. } if mid == media_id))
        .collect();
    matching.sort_by(|a, b| a.start_sec.total_cmp(&b.start_sec));

    if matching.is_empty() {
        return words.to_vec();
    }

    let mut mapped = Vec::new();
    let last_clip_idx = matching.len() - 1;
    for (idx, clip) in matching.iter().enumerate() {
        let src_start = clip.source_in_sec;
        let src_end = clip.source_in_sec + clip.duration_sec;
        for w in words {
            // Assign split-boundary words strictly by midpoint so no word is duplicated across adjacent split clips.
            let mid = 0.5 * (w.start_sec + w.end_sec);
            let in_clip = if idx == last_clip_idx {
                mid >= src_start && mid <= src_end
            } else {
                mid >= src_start && mid < src_end
            };
            if in_clip {
                let clamped_s = w.start_sec.max(src_start);
                let clamped_e = w.end_sec.min(src_end);
                if clamped_e > clamped_s {
                    mapped.push(TranscriptWord {
                        word: w.word.clone(),
                        start_sec: clip.start_sec + (clamped_s - src_start),
                        end_sec: clip.start_sec + (clamped_e - src_start),
                    });
                }
            }
        }
    }
    mapped.sort_by(|a, b| a.start_sec.total_cmp(&b.start_sec));
    mapped
}

fn strip_waveform_peaks(asset_val: &mut Value) {
    if let Some(intel) = asset_val
        .get_mut("intelligence")
        .and_then(Value::as_object_mut)
    {
        let count = intel
            .remove("waveform_peaks")
            .and_then(|v| v.as_array().map(Vec::len))
            .unwrap_or(0);
        intel.insert("waveform_peaks_count".to_string(), json!(count));
    }
}

fn compact_media_asset_json(asset: &MediaAsset) -> Value {
    let mut val = serde_json::to_value(asset).unwrap_or_default();
    strip_waveform_peaks(&mut val);
    val
}

fn compact_project_json(proj: &Project) -> Value {
    let mut val = serde_json::to_value(proj).unwrap_or_default();
    if let Some(pool) = val.get_mut("media_pool").and_then(Value::as_array_mut) {
        for item in pool {
            strip_waveform_peaks(item);
        }
    }
    if let Some(log) = val.get_mut("activity_log").and_then(Value::as_array_mut)
        && log.len() > 20
    {
        let keep_from = log.len() - 20;
        log.drain(..keep_from);
    }
    val
}

fn is_cjk_char(c: char) -> bool {
    matches!(
        c as u32,
        0x3040..=0x309F
            | 0x30A0..=0x30FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0xAC00..=0xD7AF
    )
}

fn is_closing_quote_or_bracket(c: char) -> bool {
    matches!(
        c,
        '"' | '\'' | '”' | '’' | ')' | ']' | '」' | '』' | '》' | '）' | '»'
    )
}

fn is_clause_punctuation(c: char) -> bool {
    matches!(
        c,
        '.' | '!'
            | '?'
            | ':'
            | ';'
            | ','
            | '…'
            | '—'
            | '–'
            | '。'
            | '！'
            | '？'
            | '，'
            | '、'
            | '；'
            | '：'
            | '،'
            | '؛'
            | '؟'
    )
}

fn ends_with_clause_punctuation(word: &str) -> bool {
    let trimmed = word.trim_end_matches(is_closing_quote_or_bracket);
    let Some(last) = trimmed.chars().next_back() else {
        return false;
    };
    if !is_clause_punctuation(last) {
        return false;
    }
    if last == '.' {
        let lower = trimmed
            .trim_start_matches(['"', '\'', '“', '‘', '(', '['])
            .to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "dr." | "mr." | "ms." | "mrs." | "vs." | "e.g." | "i.e." | "etc."
        ) {
            return false;
        }
        let bytes = lower.as_bytes();
        if bytes.len() == 2 && bytes[0].is_ascii_digit() && bytes[1] == b'.' {
            return false;
        }
    }
    true
}

fn split_long_cjk_token(token: &str) -> Vec<String> {
    let chars: Vec<char> = token.chars().collect();
    if chars.len() <= 6 || !chars.iter().any(|&c| is_cjk_char(c)) {
        return vec![token.to_string()];
    }

    // First break at embedded clause punctuation (keeping trailing closing quotes/brackets attached).
    let mut clauses: Vec<Vec<char>> = Vec::new();
    let mut cur: Vec<char> = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        cur.push(ch);
        i += 1;
        if is_clause_punctuation(ch) {
            while i < chars.len() && is_closing_quote_or_bracket(chars[i]) {
                cur.push(chars[i]);
                i += 1;
            }
            clauses.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        clauses.push(cur);
    }

    // Balance any remaining >6-char clause into <=4-char chunks so captions fit safe zones cleanly.
    let mut out: Vec<String> = Vec::new();
    for clause in clauses {
        if clause.len() <= 6 {
            out.push(clause.into_iter().collect());
            continue;
        }
        let num_chunks = clause.len().div_ceil(4);
        let base = clause.len() / num_chunks;
        let rem = clause.len() % num_chunks;
        let mut pos = 0usize;
        for chunk_idx in 0..num_chunks {
            let size = base + usize::from(chunk_idx < rem);
            out.push(clause[pos..pos + size].iter().collect());
            pos += size;
        }
    }
    out
}

fn tokenize_caption_text(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for raw in text.split_whitespace() {
        tokens.extend(split_long_cjk_token(raw));
    }
    tokens
}

fn build_caption_clips_from_bursts(
    mut bursts: Vec<Vec<&TranscriptWord>>,
    id_prefix: &str,
    cap_y: f64,
    preset: CaptionPreset,
    font_family: &str,
    highlight_color: &str,
) -> Vec<Clip> {
    bursts.retain(|b| !b.is_empty());
    bursts.sort_by(|a, b| a[0].start_sec.total_cmp(&b[0].start_sec));

    let mut clips = Vec::with_capacity(bursts.len());
    for (idx, chunk) in bursts.iter().enumerate() {
        let Some(first) = chunk.first() else { continue };
        let Some(last) = chunk.last() else { continue };
        let start_sec = first.start_sec.max(0.0);
        let mut duration_sec = (last.end_sec - start_sec).max(0.25);
        if let Some(next_start) = bursts
            .get(idx + 1)
            .and_then(|next| next.first())
            .map(|w| w.start_sec)
        {
            let avail = (next_start - start_sec).max(0.01);
            let max_no_overlap = if avail > 0.07 {
                avail - 0.02
            } else if avail > 0.02 {
                (avail - 0.01).max(0.01)
            } else {
                avail
            };
            duration_sec = duration_sec.min(max_no_overlap);
        }
        let clip_end = start_sec + duration_sec;
        let label = chunk
            .iter()
            .map(|w| w.word.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let words = chunk
            .iter()
            .map(|w| {
                let ws = w.start_sec.clamp(start_sec, clip_end);
                let we = w.end_sec.clamp(ws, clip_end);
                CaptionWord {
                    text: w.word.clone(),
                    start_sec: ws,
                    end_sec: we,
                }
            })
            .collect();
        clips.push(Clip {
            id: format!("{id_prefix}-{idx}-{}", uuid::Uuid::new_v4().simple()),
            label,
            start_sec,
            duration_sec,
            source_in_sec: 0.0,
            transform: Transform2D {
                y: cap_y,
                ..Transform2D::default()
            },
            audio: AudioSettings::default(),
            payload: ClipKind::Caption {
                caption: CaptionBlock {
                    words,
                    preset,
                    font_family: font_family.to_string(),
                    highlight_color: highlight_color.to_string(),
                },
            },
        });
    }
    clips
}

fn split_caption_bursts(
    words: &[TranscriptWord],
    words_per_burst: usize,
    silence_gap_sec: f64,
) -> Vec<Vec<&TranscriptWord>> {
    let max_w = words_per_burst.max(1);
    // First group into natural phrases separated by silence gaps or clause/sentence punctuation.
    let mut phrases: Vec<Vec<&TranscriptWord>> = Vec::new();
    let mut current: Vec<&TranscriptWord> = Vec::new();
    for w in words {
        let gap_exceeded = current
            .last()
            .is_some_and(|prev| w.start_sec - prev.end_sec > silence_gap_sec);
        if !current.is_empty() && gap_exceeded {
            phrases.push(std::mem::take(&mut current));
        }
        current.push(w);
        if ends_with_clause_punctuation(&w.word) {
            phrases.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        phrases.push(current);
    }

    // Balance any phrase longer than `max_w` evenly (e.g., 4 words -> 2+2 instead of 3+1).
    let mut bursts: Vec<Vec<&TranscriptWord>> = Vec::new();
    for phrase in phrases {
        if phrase.len() <= max_w {
            bursts.push(phrase);
            continue;
        }
        let num_chunks = phrase.len().div_ceil(max_w);
        let base = phrase.len() / num_chunks;
        let rem = phrase.len() % num_chunks;
        let mut cursor = 0usize;
        for chunk_idx in 0..num_chunks {
            let size = base + usize::from(chunk_idx < rem);
            bursts.push(phrase[cursor..cursor + size].to_vec());
            cursor += size;
        }
    }
    bursts
}

pub async fn execute_tool(
    backend: &dyn SolarisBackend,
    raw_name: &str,
    args: Value,
    meta: CommandMeta,
) -> Result<ToolOutput, AgentError> {
    let name = normalize_tool_name(raw_name.trim());
    match name {
        "project.list" => {
            let list = backend.list_projects().await?;
            Ok(ToolOutput {
                text: format!("Found {} projects", list.len()),
                structured: json!({ "projects": list }),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "project.create" => {
            let pname = req_str(&args, "name")?.to_string();
            let aspect: Option<AspectPreset> = parse_optional_enum(&args, "aspect_preset")?;
            let brand_id = args
                .get("brand_playbook_id")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let prompt = args
                .get("prompt")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let template_id = args
                .get("template_id")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let proj = backend
                .create_project(pname, aspect, brand_id, prompt, template_id)
                .await?;
            Ok(ToolOutput {
                text: format!("Created project '{}' ({})", proj.name, proj.id),
                structured: serde_json::to_value(&proj).unwrap_or_default(),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "project.run_qa" => {
            let pid = req_str(&args, "project_id")?;
            // Fail cleanly on missing project IDs instead of returning an Ok report
            let _ = backend.get_project(pid).await?;
            let auto_fix = args
                .get("auto_fix")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let cancel = tokio_util::sync::CancellationToken::new();
            let report = run_qa_pass(backend, None, None, pid, auto_fix, &cancel, None).await;
            Ok(ToolOutput {
                text: format!(
                    "QA score {}/100 ({} passed, {} auto-fixes, {} frames inspected)",
                    report.score,
                    if report.passed {
                        "PASSED"
                    } else {
                        "NEEDS ATTENTION"
                    },
                    report.auto_fixes_applied,
                    report.frames_inspected
                ),
                structured: serde_json::to_value(&report).unwrap_or_default(),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "project.get" => {
            let pid = req_str(&args, "project_id")?;
            let proj = backend.get_project(pid).await?;
            Ok(ToolOutput {
                text: format!("Project '{}' rev {}", proj.name, proj.revision),
                structured: compact_project_json(&proj),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "project.undo" => {
            let pid = req_str(&args, "project_id")?;
            let proj = backend.undo(pid).await?;
            Ok(ToolOutput {
                text: format!("Undid last op (now rev {})", proj.revision),
                structured: json!({ "revision": proj.revision }),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "project.redo" => {
            let pid = req_str(&args, "project_id")?;
            let proj = backend.redo(pid).await?;
            Ok(ToolOutput {
                text: format!("Redid op (now rev {})", proj.revision),
                structured: json!({ "revision": proj.revision }),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "project.lock_track" => {
            let pid = req_str(&args, "project_id")?;
            let tid = req_str(&args, "track_id")?;
            backend
                .acquire_track_lock(pid, tid, &meta.agent_id, meta.role)
                .await?;
            Ok(ToolOutput {
                text: format!("Locked track {tid}"),
                structured: json!({ "locked": tid }),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "project.unlock_track" => {
            let pid = req_str(&args, "project_id")?;
            let tid = req_str(&args, "track_id")?;
            backend.release_track_lock(pid, tid, &meta.agent_id).await?;
            Ok(ToolOutput {
                text: format!("Unlocked track {tid}"),
                structured: json!({ "unlocked": tid }),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "media.import" => {
            let pid = req_str(&args, "project_id")?;
            let items = args
                .get("paths")
                .and_then(|v| v.as_array())
                .ok_or_else(|| AgentError::ToolError("missing paths array".to_string()))?;
            let mut pairs = Vec::with_capacity(items.len());
            for item in items {
                let p = item.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
                    AgentError::ToolError("path entry missing string path".to_string())
                })?;
                let hint = item
                    .get("role_hint")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                pairs.push((p.to_string(), hint));
            }
            let assets = backend.import_media_paths(pid, pairs).await?;
            Ok(ToolOutput {
                text: format!("Imported {} media asset(s)", assets.len()),
                structured: json!({ "media": assets }),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "media.get_manifest" => {
            let pid = req_str(&args, "project_id")?;
            let proj = backend.get_project(pid).await?;
            if let Some(mid) = args.get("media_id").and_then(|v| v.as_str()) {
                let asset = proj
                    .media_pool
                    .into_iter()
                    .find(|m| m.id == mid)
                    .ok_or_else(|| AgentError::ToolError(format!("media '{mid}' not found")))?;
                Ok(ToolOutput {
                    text: format!("Manifest for '{}'", asset.name),
                    structured: compact_media_asset_json(&asset),
                    image_png_base64: None,
                    images_png_base64: Vec::new(),
                })
            } else {
                let compact_pool: Vec<Value> = proj
                    .media_pool
                    .iter()
                    .map(compact_media_asset_json)
                    .collect();
                Ok(ToolOutput {
                    text: format!("{} media asset(s) in pool", compact_pool.len()),
                    structured: json!({ "media_pool": compact_pool }),
                    image_png_base64: None,
                    images_png_base64: Vec::new(),
                })
            }
        }
        "media.transcribe" => {
            let pid = req_str(&args, "project_id")?;
            let mid = req_str(&args, "media_id")?;
            let lang = args
                .get("language")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let words = backend.transcribe_media(pid, mid, lang).await?;
            Ok(ToolOutput {
                text: format!("Transcribed {} words", words.len()),
                structured: json!({ "word_count": words.len(), "words": words }),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "media.extract_keyframes" => {
            let pid = req_str(&args, "project_id")?;
            let mid = req_str(&args, "media_id")?;
            let ts: Vec<f64> = args
                .get("timestamps_sec")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|x| x.as_f64()).collect())
                .unwrap_or_else(|| vec![0.5, 2.5, 5.0]);
            let frames = backend.extract_keyframes(pid, mid, ts).await?;
            let metadata: Vec<Value> = frames
                .iter()
                .map(|f| json!({ "time_sec": f.time_sec }))
                .collect();
            let images: Vec<String> = frames.into_iter().map(|f| f.png_base64).collect();
            Ok(ToolOutput {
                text: format!("Extracted {} keyframe(s)", metadata.len()),
                structured: json!({ "keyframes": metadata }),
                image_png_base64: None,
                images_png_base64: images,
            })
        }
        "timeline.place_clip" => {
            let pid = req_str(&args, "project_id")?;
            let track_id = req_str(&args, "track_id")?.to_string();
            let clip_id = req_str(&args, "clip_id")?.to_string();
            let media_id = req_str(&args, "media_id")?.to_string();
            let start_sec = req_f64(&args, "start_sec")?;
            let duration_sec = req_f64(&args, "duration_sec")?;
            let source_in_sec = args
                .get("source_in_sec")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let label = args
                .get("label")
                .and_then(|v| v.as_str())
                .unwrap_or(&clip_id)
                .to_string();
            let ken_burns = args
                .get("ken_burns")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let scale = args.get("scale").and_then(|v| v.as_f64()).unwrap_or(1.0);
            let gain_db = args.get("gain_db").and_then(|v| v.as_f64()).unwrap_or(0.0);

            if !scale.is_finite() || scale <= 0.0 {
                return Err(AgentError::ToolError(
                    "scale must be a finite number > 0".to_string(),
                ));
            }
            if !source_in_sec.is_finite() || source_in_sec < 0.0 || !gain_db.is_finite() {
                return Err(AgentError::ToolError(
                    "source_in_sec and gain_db must be finite (source_in_sec >= 0)".to_string(),
                ));
            }

            let proj = backend.get_project(pid).await?;
            let media = proj
                .media_pool
                .iter()
                .find(|m| m.id == media_id)
                .ok_or_else(|| {
                    AgentError::ToolError(format!(
                        "media_id '{media_id}' does not exist in project media_pool"
                    ))
                })?;
            let track = proj
                .tracks
                .iter()
                .find(|t| t.id == track_id)
                .ok_or_else(|| AgentError::ToolError(format!("track '{track_id}' not found")))?;
            match (track.kind, media.kind) {
                (TrackKind::ARoll | TrackKind::BRoll, MediaKind::Video | MediaKind::Image) => {}
                (TrackKind::Voice | TrackKind::Music, MediaKind::Audio | MediaKind::Video) => {}
                _ => {
                    return Err(AgentError::ToolError(format!(
                        "media '{}' ({:?}) is incompatible with track '{}' ({:?})",
                        media.name, media.kind, track.id, track.kind
                    )));
                }
            }

            let clip = Clip {
                id: clip_id,
                label,
                start_sec,
                duration_sec,
                source_in_sec,
                transform: Transform2D {
                    scale,
                    ..Transform2D::default()
                },
                audio: AudioSettings {
                    gain_db,
                    ..AudioSettings::default()
                },
                payload: ClipKind::Media {
                    media_id,
                    ken_burns,
                },
            };
            let (res, _) = backend
                .apply_op(pid, TimelineCommand::PlaceClip { track_id, clip }, meta)
                .await?;
            Ok(ToolOutput {
                text: res.summary.clone(),
                structured: serde_json::to_value(&res).unwrap_or_default(),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "timeline.split_clip" => {
            let pid = req_str(&args, "project_id")?;
            let track_id = req_str(&args, "track_id")?.to_string();
            let clip_id = req_str(&args, "clip_id")?.to_string();
            let split_at_timeline_sec = req_f64(&args, "split_at_timeline_sec")?;
            let right_clip_id = req_str(&args, "right_clip_id")?.to_string();
            let (res, _) = backend
                .apply_op(
                    pid,
                    TimelineCommand::SplitClip {
                        track_id,
                        clip_id,
                        split_at_timeline_sec,
                        right_clip_id,
                    },
                    meta,
                )
                .await?;
            Ok(ToolOutput {
                text: res.summary.clone(),
                structured: serde_json::to_value(&res).unwrap_or_default(),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "timeline.trim_clip" => {
            let pid = req_str(&args, "project_id")?;
            let track_id = req_str(&args, "track_id")?.to_string();
            let clip_id = req_str(&args, "clip_id")?.to_string();
            let (res, _) = backend
                .apply_op(
                    pid,
                    TimelineCommand::TrimClip {
                        track_id,
                        clip_id,
                        new_start_sec: args.get("new_start_sec").and_then(|v| v.as_f64()),
                        new_duration_sec: args.get("new_duration_sec").and_then(|v| v.as_f64()),
                        new_source_in_sec: args.get("new_source_in_sec").and_then(|v| v.as_f64()),
                    },
                    meta,
                )
                .await?;
            Ok(ToolOutput {
                text: res.summary.clone(),
                structured: serde_json::to_value(&res).unwrap_or_default(),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "timeline.delete_clip" => {
            let pid = req_str(&args, "project_id")?;
            let track_id = req_str(&args, "track_id")?.to_string();
            let clip_id = req_str(&args, "clip_id")?.to_string();
            let (res, _) = backend
                .apply_op(pid, TimelineCommand::DeleteClip { track_id, clip_id }, meta)
                .await?;
            Ok(ToolOutput {
                text: res.summary.clone(),
                structured: serde_json::to_value(&res).unwrap_or_default(),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "timeline.set_transform" => {
            let pid = req_str(&args, "project_id")?;
            let track_id = req_str(&args, "track_id")?.to_string();
            let clip_id = req_str(&args, "clip_id")?.to_string();
            let proj = backend.get_project(pid).await?;
            let existing = proj
                .tracks
                .iter()
                .find(|t| t.id == track_id)
                .and_then(|t| t.clips.iter().find(|c| c.id == clip_id))
                .map(|c| c.transform.clone())
                .unwrap_or_default();
            let transform = Transform2D {
                x: args.get("x").and_then(|v| v.as_f64()).unwrap_or(existing.x),
                y: args.get("y").and_then(|v| v.as_f64()).unwrap_or(existing.y),
                scale: args
                    .get("scale")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(existing.scale),
                rotation_deg: args
                    .get("rotation_deg")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(existing.rotation_deg),
                opacity: args
                    .get("opacity")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(existing.opacity),
                smart_reframe: existing.smart_reframe,
            };
            let (res, _) = backend
                .apply_op(
                    pid,
                    TimelineCommand::UpdateClipTransform {
                        track_id,
                        clip_id,
                        transform,
                    },
                    meta,
                )
                .await?;
            Ok(ToolOutput {
                text: res.summary.clone(),
                structured: serde_json::to_value(&res).unwrap_or_default(),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "color.set_grade" => {
            let pid = req_str(&args, "project_id")?;
            let proj = backend.get_project(pid).await?;
            let track_id = args
                .get("track_id")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let clip_id = args
                .get("clip_id")
                .and_then(|v| v.as_str())
                .map(str::to_string);

            let mut grade: ColorGrade = match (&track_id, &clip_id) {
                (Some(tid), Some(cid)) => proj
                    .tracks
                    .iter()
                    .find(|t| t.id == *tid)
                    .and_then(|t| t.clips.iter().find(|c| c.id == *cid))
                    .and_then(|c| match &c.payload {
                        ClipKind::Grade { grade } => Some(grade.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| proj.active_grade.clone()),
                _ => proj.active_grade,
            };

            apply_rgb_patch(&mut grade.lift, args.get("lift"));
            apply_rgb_patch(&mut grade.gamma, args.get("gamma"));
            apply_rgb_patch(&mut grade.gain, args.get("gain"));

            if let Some(v) = args.get("contrast").and_then(|v| v.as_f64()) {
                grade.contrast = v;
            }
            if let Some(v) = args.get("saturation").and_then(|v| v.as_f64()) {
                grade.saturation = v;
            }
            if let Some(v) = args.get("exposure").and_then(|v| v.as_f64()) {
                grade.exposure = v;
            }
            if let Some(v) = args.get("temp").and_then(|v| v.as_f64()) {
                grade.temp = v;
            }
            if let Some(v) = args.get("tint").and_then(|v| v.as_f64()) {
                grade.tint = v;
            }
            if let Some(lut_val) = args.get("lut_path") {
                if lut_val.is_null() {
                    grade.lut_path = None;
                } else if let Some(s) = lut_val.as_str() {
                    grade.lut_path = (!s.trim().is_empty()).then(|| s.to_string());
                }
            }
            if let Some(v) = args.get("lut_intensity").and_then(|v| v.as_f64()) {
                grade.lut_intensity = v;
            }
            let (res, _) = backend
                .apply_op(
                    pid,
                    TimelineCommand::SetColorGrade {
                        grade,
                        track_id,
                        clip_id,
                    },
                    meta,
                )
                .await?;
            Ok(ToolOutput {
                text: res.summary.clone(),
                structured: serde_json::to_value(&res).unwrap_or_default(),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "color.get_scope_stats" => {
            let pid = req_str(&args, "project_id")?;
            let proj = backend.get_project(pid).await?;
            let brand = backend.get_brand(&proj.brand_playbook_id).await.ok();
            let log_assets: Vec<String> = proj
                .media_pool
                .iter()
                .filter(|m| m.intelligence.is_log_footage)
                .map(|m| m.name.clone())
                .collect();
            let stats = json!({
                "active_grade": proj.active_grade,
                "brand_default_lut": brand.and_then(|b| b.default_lut),
                "log_footage_assets": log_assets,
            });
            Ok(ToolOutput {
                text: format!("Color scope telemetry for '{}'", proj.name),
                structured: stats,
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "captions.generate_from_transcript" => {
            let pid = req_str(&args, "project_id")?;
            let proj = backend.get_project(pid).await?;
            let brand = backend.get_brand(&proj.brand_playbook_id).await.ok();
            let preset: CaptionPreset =
                parse_optional_enum(&args, "preset")?.unwrap_or(CaptionPreset::NeonGlow);
            let words_per_burst = args
                .get("words_per_burst")
                .and_then(|v| v.as_u64())
                .unwrap_or(3)
                .clamp(1, 6) as usize;

            // Map against BOTH V1 (A-Roll) and A1 (Voiceover) tracks, deduplicating identical
            // (media_id, start_sec, source_in_sec) spans so words are never double-emitted.
            let raw_speech_clips: Vec<&Clip> = proj
                .tracks
                .iter()
                .filter(|t| t.id == "v1" || t.id == "a1")
                .flat_map(|t| t.clips.iter())
                .collect();
            let mut seen_spans: HashSet<(String, i64, i64)> = HashSet::new();
            let mut speech_clips: Vec<&Clip> = Vec::new();
            for clip in raw_speech_clips {
                if let ClipKind::Media { media_id, .. } = &clip.payload {
                    let key = (
                        media_id.clone(),
                        (clip.start_sec * 1000.0).round() as i64,
                        (clip.source_in_sec * 1000.0).round() as i64,
                    );
                    if seen_spans.insert(key) {
                        speech_clips.push(clip);
                    }
                }
            }

            let target_assets: Vec<&MediaAsset> =
                if let Some(mid) = args.get("media_id").and_then(|v| v.as_str()) {
                    let Some(asset) = proj.media_pool.iter().find(|m| m.id == mid) else {
                        return Err(AgentError::ToolError(format!(
                            "media '{mid}' not found in project media_pool"
                        )));
                    };
                    if asset.intelligence.transcript.is_empty() {
                        return Err(AgentError::ToolError(
                            "media asset has an empty transcript".to_string(),
                        ));
                    }
                    vec![asset]
                } else {
                    let mut seen_media: HashSet<&str> = HashSet::new();
                    let mut placed_transcribed: Vec<&MediaAsset> = Vec::new();
                    for clip in &speech_clips {
                        if let ClipKind::Media { media_id, .. } = &clip.payload
                            && seen_media.insert(media_id.as_str())
                            && let Some(asset) = proj.media_pool.iter().find(|m| {
                                m.id == *media_id && !m.intelligence.transcript.is_empty()
                            })
                        {
                            placed_transcribed.push(asset);
                        }
                    }
                    if !placed_transcribed.is_empty() {
                        placed_transcribed
                    } else {
                        proj.media_pool
                            .iter()
                            .filter(|m| !m.intelligence.transcript.is_empty())
                            .collect()
                    }
                };

            if target_assets.is_empty() {
                return Err(AgentError::ToolError(
                    "no media asset with transcript found; run media.transcribe first".to_string(),
                ));
            }

            let mut timeline_words: Vec<TranscriptWord> = Vec::new();
            for asset in &target_assets {
                let mapped = map_transcript_to_timeline(
                    &asset.intelligence.transcript,
                    &asset.id,
                    &speech_clips,
                );
                timeline_words.extend(mapped);
            }
            timeline_words.sort_by(|a, b| a.start_sec.total_cmp(&b.start_sec));

            if timeline_words.is_empty() {
                return Err(AgentError::ToolError(
                    "no transcript words mapped to placed V1/A1 clip ranges; existing T1 captions were preserved".to_string(),
                ));
            }

            let font_family = brand
                .as_ref()
                .map(|b| b.tokens.font_display.clone())
                .unwrap_or_else(|| "Sora".to_string());
            let highlight_color = brand
                .as_ref()
                .map(|b| b.tokens.accent_secondary.clone())
                .unwrap_or_else(|| "#F7B733".to_string());

            let bursts = split_caption_bursts(&timeline_words, words_per_burst, 0.6);
            let clips = build_caption_clips_from_bursts(
                bursts,
                "cap",
                0.20,
                preset,
                &font_family,
                &highlight_color,
            );
            if clips.is_empty() {
                return Err(AgentError::ToolError(
                    "captions.generate_from_transcript produced 0 caption clips; existing T1 captions were preserved".to_string(),
                ));
            }

            // Replace existing caption clips on T1 so regenerating never stacks duplicates
            if let Some(t1) = proj.tracks.iter().find(|t| t.id == "t1") {
                let existing_cap_ids: Vec<String> = t1
                    .clips
                    .iter()
                    .filter(|c| matches!(c.payload, ClipKind::Caption { .. }))
                    .map(|c| c.id.clone())
                    .collect();
                for old_id in existing_cap_ids {
                    backend
                        .apply_op(
                            pid,
                            TimelineCommand::DeleteClip {
                                track_id: "t1".to_string(),
                                clip_id: old_id,
                            },
                            meta.clone(),
                        )
                        .await?;
                }
            }

            let mut placed = 0usize;
            for clip in clips {
                backend
                    .apply_op(
                        pid,
                        TimelineCommand::PlaceClip {
                            track_id: "t1".to_string(),
                            clip,
                        },
                        meta.clone(),
                    )
                    .await?;
                placed += 1;
            }

            Ok(ToolOutput {
                text: format!("Placed {placed} caption burst(s) on T1"),
                structured: json!({ "placed_count": placed }),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "captions.set_style" => {
            let pid = req_str(&args, "project_id")?;
            let preset: CaptionPreset = args
                .get("preset")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .ok_or_else(|| AgentError::ToolError("invalid caption preset".to_string()))?;
            let proj = backend.get_project(pid).await?;
            let Some(t1) = proj.tracks.iter().find(|t| t.id == "t1") else {
                return Err(AgentError::ToolError("T1 track missing".to_string()));
            };
            let mut updated = 0usize;
            for clip in &t1.clips {
                if let ClipKind::Caption { caption } = &clip.payload {
                    let mut next = caption.clone();
                    next.preset = preset;
                    if let Some(f) = args.get("font_family").and_then(|v| v.as_str()) {
                        next.font_family = f.to_string();
                    }
                    if let Some(c) = args.get("highlight_color").and_then(|v| v.as_str()) {
                        next.highlight_color = c.to_string();
                    }
                    backend
                        .apply_op(
                            pid,
                            TimelineCommand::UpdateCaptionClip {
                                track_id: "t1".to_string(),
                                clip_id: clip.id.clone(),
                                caption: next,
                            },
                            meta.clone(),
                        )
                        .await?;
                    updated += 1;
                }
            }
            Ok(ToolOutput {
                text: format!("Updated style on {updated} caption clip(s)"),
                structured: json!({ "updated_count": updated }),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "captions.place_script" => {
            let pid = req_str(&args, "project_id")?;
            let proj = backend.get_project(pid).await?;
            let brand = backend.get_brand(&proj.brand_playbook_id).await.ok();
            let preset: CaptionPreset =
                parse_optional_enum(&args, "preset")?.unwrap_or(CaptionPreset::NeonGlow);
            let words_per_burst = args
                .get("words_per_burst")
                .and_then(|v| v.as_u64())
                .unwrap_or(3)
                .clamp(1, 6) as usize;
            let cap_y = args
                .get("y")
                .and_then(|v| v.as_f64())
                .filter(|v| v.is_finite())
                .unwrap_or(0.24)
                .clamp(-0.32, 0.32);
            let replace_existing = match args.get("replace_existing") {
                None | Some(Value::Null) => true,
                Some(Value::Bool(b)) => *b,
                Some(_) => {
                    return Err(AgentError::ToolError(
                        "invalid argument 'replace_existing': expected boolean".to_string(),
                    ));
                }
            };

            let segments = args
                .get("segments")
                .and_then(|v| v.as_array())
                .filter(|a| !a.is_empty())
                .ok_or_else(|| {
                    AgentError::ToolError(
                        "captions.place_script requires a non-empty 'segments' array".to_string(),
                    )
                })?;

            // Pre-validate ALL segments BEFORE modifying T1 so invalid input never deletes existing captions.
            let truncated_segments = segments.len().saturating_sub(64);
            struct ParsedSegment {
                start_sec: f64,
                dur_sec: f64,
                tokens: Vec<String>,
            }
            let mut parsed_segments: Vec<ParsedSegment> = Vec::new();
            for (seg_idx, seg) in segments.iter().enumerate() {
                let start_sec = seg
                    .get("start_sec")
                    .and_then(|v| v.as_f64())
                    .filter(|v| v.is_finite() && *v >= 0.0)
                    .ok_or_else(|| {
                        AgentError::ToolError(format!(
                            "segment {seg_idx} missing valid start_sec >= 0"
                        ))
                    })?;
                let dur_sec = seg
                    .get("duration_sec")
                    .and_then(|v| v.as_f64())
                    .filter(|v| v.is_finite() && *v > 0.05)
                    .ok_or_else(|| {
                        AgentError::ToolError(format!(
                            "segment {seg_idx} missing valid duration_sec > 0.05"
                        ))
                    })?;
                if !(start_sec + dur_sec).is_finite() {
                    return Err(AgentError::ToolError(format!(
                        "segment {seg_idx} has non-finite end time (start_sec + duration_sec)"
                    )));
                }
                if seg_idx >= 64 {
                    continue;
                }
                let text = seg
                    .get("text")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .unwrap_or("");
                let tokens = tokenize_caption_text(text);
                if tokens.is_empty() {
                    continue;
                }
                parsed_segments.push(ParsedSegment {
                    start_sec,
                    dur_sec,
                    tokens,
                });
            }

            if parsed_segments.is_empty() {
                return Err(AgentError::ToolError(
                    "captions.place_script produced 0 words from segments".to_string(),
                ));
            }
            parsed_segments.sort_by(|a, b| a.start_sec.total_cmp(&b.start_sec));

            // Prevent overlapping segments: trim earlier segment when there is room, else advance later start.
            for i in 1..parsed_segments.len() {
                let prev_start = parsed_segments[i - 1].start_sec;
                let prev_end = prev_start + parsed_segments[i - 1].dur_sec;
                if parsed_segments[i].start_sec < prev_end {
                    if parsed_segments[i].start_sec >= prev_start + 0.10 {
                        parsed_segments[i - 1].dur_sec =
                            (parsed_segments[i].start_sec - prev_start).max(0.06);
                    } else {
                        parsed_segments[i].start_sec = prev_end;
                    }
                }
            }

            // Build word timings strictly clamped within each segment's [start_sec, seg_end].
            let mut segment_word_groups: Vec<Vec<TranscriptWord>> =
                Vec::with_capacity(parsed_segments.len());
            for seg in &parsed_segments {
                let seg_end = seg.start_sec + seg.dur_sec;
                let seg_end_r = (seg_end * 1000.0).round() / 1000.0;
                let step = seg.dur_sec / (seg.tokens.len() as f64);
                let mut seg_words = Vec::with_capacity(seg.tokens.len());
                for (wi, tok) in seg.tokens.iter().enumerate() {
                    let ws = seg.start_sec + (wi as f64) * step;
                    let we = if wi + 1 == seg.tokens.len() {
                        seg_end
                    } else {
                        (ws + step).min(seg_end)
                    };
                    let ws_r = ((ws * 1000.0).round() / 1000.0).min(seg_end_r);
                    let we_r = ((we * 1000.0).round() / 1000.0).clamp(ws_r, seg_end_r);
                    seg_words.push(TranscriptWord {
                        word: tok.clone(),
                        start_sec: ws_r,
                        end_sec: we_r,
                    });
                }
                segment_word_groups.push(seg_words);
            }

            let font_family = brand
                .as_ref()
                .map(|b| b.tokens.font_display.clone())
                .unwrap_or_else(|| "Sora".to_string());
            let highlight_color = brand
                .as_ref()
                .map(|b| b.tokens.accent_secondary.clone())
                .unwrap_or_else(|| "#F7B733".to_string());

            // Pre-build and validate all bursts & clips BEFORE touching existing T1 clips.
            let bursts: Vec<Vec<&TranscriptWord>> = segment_word_groups
                .iter()
                .flat_map(|seg_words| split_caption_bursts(seg_words, words_per_burst, 0.35))
                .collect();
            let clips = build_caption_clips_from_bursts(
                bursts,
                "cap-script",
                cap_y,
                preset,
                &font_family,
                &highlight_color,
            );
            if clips.is_empty() {
                return Err(AgentError::ToolError(
                    "captions.place_script produced 0 caption clips".to_string(),
                ));
            }

            if replace_existing && let Some(t1) = proj.tracks.iter().find(|t| t.id == "t1") {
                let existing_cap_ids: Vec<String> = t1
                    .clips
                    .iter()
                    .filter(|c| matches!(c.payload, ClipKind::Caption { .. }))
                    .map(|c| c.id.clone())
                    .collect();
                for old_id in existing_cap_ids {
                    backend
                        .apply_op(
                            pid,
                            TimelineCommand::DeleteClip {
                                track_id: "t1".to_string(),
                                clip_id: old_id,
                            },
                            meta.clone(),
                        )
                        .await?;
                }
            }

            let mut placed = 0usize;
            for clip in clips {
                backend
                    .apply_op(
                        pid,
                        TimelineCommand::PlaceClip {
                            track_id: "t1".to_string(),
                            clip,
                        },
                        meta.clone(),
                    )
                    .await?;
                placed += 1;
            }

            Ok(ToolOutput {
                text: format!("Placed {placed} scripted caption burst(s) on T1"),
                structured: json!({
                    "placed_count": placed,
                    "truncated_segments": truncated_segments,
                }),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "assets.search" => {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();
            let category = args
                .get("category")
                .and_then(|v| v.as_str())
                .map(|s| s.to_lowercase());
            let all = backend.list_assets().await?;
            let filtered: Vec<SavedCodeAsset> = all
                .into_iter()
                .filter(|a| {
                    let cat_ok = category
                        .as_ref()
                        .is_none_or(|c| a.category.to_lowercase() == *c);
                    let q_ok = query.is_empty()
                        || a.id.to_lowercase().contains(&query)
                        || a.name.to_lowercase().contains(&query)
                        || a.tags.iter().any(|t| t.to_lowercase().contains(&query));
                    cat_ok && q_ok
                })
                .collect();
            Ok(ToolOutput {
                text: format!("Found {} matching GFX asset(s)", filtered.len()),
                structured: json!({ "assets": filtered }),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "assets.save" => {
            let asset_id = req_str(&args, "id")?;
            validate_safe_id(asset_id)?;
            let html_source = req_str(&args, "html_source")?.to_string();
            let lint = lint_gfx_html(&html_source);
            if !lint.valid {
                return Err(AgentError::ToolError(format!(
                    "GFX HTML failed lint: {}",
                    lint.errors.join("; ")
                )));
            }
            let asset = SavedCodeAsset {
                id: asset_id.to_string(),
                name: req_str(&args, "name")?.to_string(),
                category: args
                    .get("category")
                    .and_then(|v| v.as_str())
                    .unwrap_or("custom")
                    .to_string(),
                tags: args
                    .get("tags")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default(),
                props_schema: args
                    .get("props_schema")
                    .cloned()
                    .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
                default_props: args
                    .get("default_props")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
                html_source,
                default_duration_sec: args
                    .get("default_duration_sec")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(3.0),
                created_at: chrono::Utc::now(),
            };
            validate_single_asset(&asset).map_err(AgentError::ToolError)?;
            let saved = backend.save_asset(asset).await?;
            Ok(ToolOutput {
                text: format!("Saved GFX asset '{}' ({})", saved.name, saved.id),
                structured: serde_json::to_value(&saved).unwrap_or_default(),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "gfx.lint_html" => {
            let html = req_str(&args, "html")?;
            let report = lint_gfx_html(html);
            Ok(ToolOutput {
                text: if report.valid {
                    "GFX HTML passed deterministic lint".to_string()
                } else {
                    format!("GFX lint failed: {}", report.errors.join("; "))
                },
                structured: serde_json::to_value(&report).unwrap_or_default(),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "gfx.update_clip" => {
            let pid = req_str(&args, "project_id")?;
            let clip_id = req_str(&args, "clip_id")?.to_string();
            let proj = backend.get_project(pid).await?;
            let existing = proj
                .tracks
                .iter()
                .find(|t| t.id == "g1")
                .and_then(|t| t.clips.iter().find(|c| c.id == clip_id))
                .cloned()
                .ok_or_else(|| AgentError::ToolError(format!("G1 clip '{clip_id}' not found")))?;
            let ClipKind::Gfx { gfx } = &existing.payload else {
                return Err(AgentError::ToolError(format!(
                    "clip '{clip_id}' is not a GFX clip"
                )));
            };
            let opt_num = |key: &str| {
                args.get(key)
                    .and_then(|v| v.as_f64())
                    .filter(|v| v.is_finite())
            };
            let label = args
                .get("label")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| existing.label.clone());
            let start_sec = opt_num("start_sec").unwrap_or(existing.start_sec);
            let duration_sec = opt_num("duration_sec").unwrap_or(existing.duration_sec);
            let mut next_gfx = gfx.clone();
            let mut inline_html = extract_inline_html(&args);
            if let Some(aid) = args
                .get("asset_id")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let found = backend
                    .list_assets()
                    .await?
                    .into_iter()
                    .find(|a| a.id == aid)
                    .ok_or_else(|| {
                        AgentError::ToolError(format!("unknown GFX asset_id '{aid}'"))
                    })?;
                if inline_html.is_none() {
                    inline_html = Some(found.html_source);
                }
                next_gfx.props = deep_merge_json(&found.default_props, &next_gfx.props);
                next_gfx.asset_id = Some(aid.to_string());
            }
            if let Some(html) = inline_html {
                let lint = lint_gfx_html(&html);
                if !lint.valid {
                    return Err(AgentError::ToolError(format!(
                        "GFX HTML failed lint: {}",
                        lint.errors.join("; ")
                    )));
                }
                next_gfx.inline_html = Some(html);
            }
            match args.get("props") {
                None | Some(Value::Null) => {}
                Some(v @ Value::Object(_)) => next_gfx.props = deep_merge_json(&next_gfx.props, v),
                Some(_) => {
                    return Err(AgentError::ToolError(
                        "invalid argument 'props': expected JSON object".to_string(),
                    ));
                }
            }
            // Replace in place under the SAME clip_id as one atomic, single-undo op.
            let (res, _) = backend
                .apply_op(
                    pid,
                    TimelineCommand::Batch {
                        commands: vec![
                            TimelineCommand::DeleteClip {
                                track_id: "g1".to_string(),
                                clip_id: clip_id.clone(),
                            },
                            TimelineCommand::PlaceGfx {
                                track_id: "g1".to_string(),
                                clip_id: clip_id.clone(),
                                label: label.clone(),
                                start_sec,
                                duration_sec,
                                gfx: next_gfx,
                            },
                            TimelineCommand::UpdateClipTransform {
                                track_id: "g1".to_string(),
                                clip_id,
                                transform: existing.transform.clone(),
                            },
                        ],
                        summary: Some(format!("updated GFX '{label}'")),
                    },
                    meta,
                )
                .await?;
            Ok(ToolOutput {
                text: res.summary.clone(),
                structured: serde_json::to_value(&res).unwrap_or_default(),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "captions.update_clip" => {
            let pid = req_str(&args, "project_id")?;
            let clip_id = req_str(&args, "clip_id")?.to_string();
            let proj = backend.get_project(pid).await?;
            let clip = proj
                .tracks
                .iter()
                .find(|t| t.id == "t1")
                .and_then(|t| t.clips.iter().find(|c| c.id == clip_id))
                .ok_or_else(|| AgentError::ToolError(format!("T1 clip '{clip_id}' not found")))?;
            let ClipKind::Caption { caption } = &clip.payload else {
                return Err(AgentError::ToolError(format!(
                    "clip '{clip_id}' is not a caption clip"
                )));
            };
            let mut next = caption.clone();
            if let Some(text) = args
                .get("text")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                next.words = retime_caption_words(&caption.words, text, clip);
            }
            if let Some(preset) = parse_optional_enum::<CaptionPreset>(&args, "preset")? {
                next.preset = preset;
            }
            if let Some(f) = args.get("font_family").and_then(|v| v.as_str()) {
                next.font_family = f.to_string();
            }
            if let Some(c) = args.get("highlight_color").and_then(|v| v.as_str()) {
                next.highlight_color = c.to_string();
            }
            let (res, _) = backend
                .apply_op(
                    pid,
                    TimelineCommand::UpdateCaptionClip {
                        track_id: "t1".to_string(),
                        clip_id,
                        caption: next,
                    },
                    meta,
                )
                .await?;
            Ok(ToolOutput {
                text: res.summary.clone(),
                structured: serde_json::to_value(&res).unwrap_or_default(),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "gfx.place" => {
            let pid = req_str(&args, "project_id")?;
            let clip_id = req_str(&args, "clip_id")?.to_string();
            let label = req_str(&args, "label")?.to_string();
            let start_sec = req_f64(&args, "start_sec")?;
            let duration_sec = req_f64(&args, "duration_sec")?;
            let asset_id = args
                .get("asset_id")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let mut props = match args.get("props") {
                None | Some(Value::Null) => json!({}),
                Some(v @ Value::Object(_)) => v.clone(),
                Some(_) => {
                    return Err(AgentError::ToolError(
                        "invalid argument 'props': expected JSON object".to_string(),
                    ));
                }
            };
            let mut inline_html = extract_inline_html(&args);

            if let Some(ref aid) = asset_id {
                let assets = backend.list_assets().await?;
                let found = assets.into_iter().find(|a| a.id == *aid).ok_or_else(|| {
                    AgentError::ToolError(format!("unknown GFX asset_id '{aid}'"))
                })?;
                if inline_html.is_none() {
                    inline_html = Some(found.html_source);
                }
                props = deep_merge_json(&found.default_props, &props);
            }

            let Some(ref html) = inline_html else {
                return Err(AgentError::ToolError(
                    "gfx.place requires either a valid asset_id or non-empty inline_html"
                        .to_string(),
                ));
            };
            let lint = lint_gfx_html(html);
            if !lint.valid {
                return Err(AgentError::ToolError(format!(
                    "GFX HTML failed lint: {}",
                    lint.errors.join("; ")
                )));
            }

            let (res, _) = backend
                .apply_op(
                    pid,
                    TimelineCommand::PlaceGfx {
                        track_id: "g1".to_string(),
                        clip_id,
                        label,
                        start_sec,
                        duration_sec,
                        gfx: GfxInstance {
                            asset_id,
                            props,
                            inline_html,
                        },
                    },
                    meta,
                )
                .await?;
            Ok(ToolOutput {
                text: res.summary.clone(),
                structured: serde_json::to_value(&res).unwrap_or_default(),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "audio.set_mix" => {
            let pid = req_str(&args, "project_id")?;
            let track_id = req_str(&args, "track_id")?.to_string();
            let clip_id = req_str(&args, "clip_id")?.to_string();
            let proj = backend.get_project(pid).await?;
            let existing = proj
                .tracks
                .iter()
                .find(|t| t.id == track_id)
                .and_then(|t| t.clips.iter().find(|c| c.id == clip_id))
                .map(|c| c.audio.clone())
                .unwrap_or_default();
            let audio = AudioSettings {
                gain_db: args
                    .get("gain_db")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(existing.gain_db),
                ducking_enabled: args
                    .get("ducking_enabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(existing.ducking_enabled),
                duck_target_db: args
                    .get("duck_target_db")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(existing.duck_target_db),
                fade_in_sec: args
                    .get("fade_in_sec")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(existing.fade_in_sec),
                fade_out_sec: args
                    .get("fade_out_sec")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(existing.fade_out_sec),
            };
            if !audio.gain_db.is_finite()
                || !audio.duck_target_db.is_finite()
                || !audio.fade_in_sec.is_finite()
                || !audio.fade_out_sec.is_finite()
                || audio.fade_in_sec < 0.0
                || audio.fade_out_sec < 0.0
            {
                return Err(AgentError::ToolError(
                    "audio.set_mix requires finite values with fade_in_sec >= 0 and fade_out_sec >= 0"
                        .to_string(),
                ));
            }
            let (res, _) = backend
                .apply_op(
                    pid,
                    TimelineCommand::SetAudioSettings {
                        track_id,
                        clip_id,
                        audio,
                    },
                    meta,
                )
                .await?;
            Ok(ToolOutput {
                text: res.summary.clone(),
                structured: serde_json::to_value(&res).unwrap_or_default(),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "audio.normalize_lufs" => {
            let pid = req_str(&args, "project_id")?;
            let target = args
                .get("target_lufs")
                .and_then(|v| v.as_f64())
                .filter(|v| v.is_finite())
                .unwrap_or(-14.0)
                .clamp(-70.0, -5.0);
            let proj = backend.get_project(pid).await?;
            let mut updated = 0usize;
            for tid in ["a1", "v1"] {
                if let Some(track) = proj.tracks.iter().find(|t| t.id == tid && !t.muted) {
                    for clip in &track.clips {
                        if clip.audio.gain_db <= -90.0 {
                            continue;
                        }
                        if let ClipKind::Media { media_id, .. } = &clip.payload
                            && let Some(asset) = proj.media_pool.iter().find(|m| m.id == *media_id)
                            && let Some(measured) = asset.intelligence.integrated_lufs
                        {
                            let delta = (target - measured).clamp(-12.0, 12.0);
                            let mut next_audio = clip.audio.clone();
                            next_audio.gain_db = (delta * 100.0).round() / 100.0;
                            backend
                                .apply_op(
                                    pid,
                                    TimelineCommand::SetAudioSettings {
                                        track_id: tid.to_string(),
                                        clip_id: clip.id.clone(),
                                        audio: next_audio,
                                    },
                                    meta.clone(),
                                )
                                .await?;
                            updated += 1;
                        }
                    }
                }
            }
            Ok(ToolOutput {
                text: format!("Normalized {updated} speech clip(s) toward {target:.1} LUFS"),
                structured: json!({ "normalized_clips": updated, "target_lufs": target }),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "audio.enable_ducking" => {
            let pid = req_str(&args, "project_id")?;
            let duck_target_db = args
                .get("duck_target_db")
                .and_then(|v| v.as_f64())
                .unwrap_or(-18.0);
            let proj = backend.get_project(pid).await?;
            let mut updated = 0usize;
            if let Some(a2) = proj.tracks.iter().find(|t| t.id == "a2") {
                for clip in &a2.clips {
                    let mut next = clip.audio.clone();
                    next.ducking_enabled = true;
                    next.duck_target_db = duck_target_db;
                    backend
                        .apply_op(
                            pid,
                            TimelineCommand::SetAudioSettings {
                                track_id: "a2".to_string(),
                                clip_id: clip.id.clone(),
                                audio: next,
                            },
                            meta.clone(),
                        )
                        .await?;
                    updated += 1;
                }
            }
            Ok(ToolOutput {
                text: format!("Enabled {duck_target_db:.1} dB ducking on {updated} A2 clip(s)"),
                structured: json!({ "updated_clips": updated, "duck_target_db": duck_target_db }),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "brand.list_playbooks" => {
            let brands = backend.list_brands().await?;
            Ok(ToolOutput {
                text: format!("Found {} brand playbook(s)", brands.len()),
                structured: json!({ "brands": brands }),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "brand.get_playbook" => {
            let bid = req_str(&args, "brand_id")?;
            let brand = backend.get_brand(bid).await?;
            Ok(ToolOutput {
                text: format!("Brand Playbook '{}'", brand.name),
                structured: serde_json::to_value(&brand).unwrap_or_default(),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "preview.seek" => {
            let pid = req_str(&args, "project_id")?;
            let time_sec = req_f64(&args, "time_sec")?.max(0.0);
            backend
                .emit_playhead_seek(pid, time_sec, &meta.agent_id, meta.role)
                .await?;
            Ok(ToolOutput {
                text: format!("Seeked preview playhead to {time_sec:.2}s"),
                structured: json!({ "time_sec": time_sec }),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        "preview.capture_frame" => {
            let pid = req_str(&args, "project_id")?;
            let time_sec = req_f64(&args, "time_sec")?.max(0.0);
            let captured = backend.capture_preview_frame(pid, time_sec).await?;
            Ok(ToolOutput {
                text: format!(
                    "Captured preview frame at {:.2}s ({}x{}): {}",
                    captured.time_sec, captured.width, captured.height, captured.note
                ),
                structured: json!({
                    "time_sec": captured.time_sec,
                    "width": captured.width,
                    "height": captured.height,
                    "note": captured.note,
                }),
                image_png_base64: Some(captured.png_base64),
                images_png_base64: Vec::new(),
            })
        }
        "render.export" => {
            let pid = req_str(&args, "project_id")?;
            let format: ExportFormat =
                parse_optional_enum(&args, "format")?.unwrap_or(ExportFormat::Mp4);
            let quality: ExportQuality =
                parse_optional_enum(&args, "quality")?.unwrap_or(ExportQuality::Standard);
            let target_lufs = args
                .get("target_lufs")
                .and_then(|v| v.as_f64())
                .filter(|v| v.is_finite())
                .unwrap_or(-14.0)
                .clamp(-70.0, -5.0);
            let output_name = args
                .get("output_name")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let job = backend
                .start_export(
                    pid,
                    ExportRequest {
                        format,
                        quality,
                        target_lufs,
                        output_name,
                    },
                )
                .await?;
            Ok(ToolOutput {
                text: format!("Started export job {} ({:?})", job.id, job.state),
                structured: serde_json::to_value(&job).unwrap_or_default(),
                image_png_base64: None,
                images_png_base64: Vec::new(),
            })
        }
        other => Err(AgentError::ToolError(format!("unknown tool '{other}'"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{BoxFuture, CapturedFrame, KeyframeResult};
    use solaris_core_ir::{
        AgentRole, BrandPlaybook, ExportJobStatus, ExportRequest, MediaIntelligenceManifest,
        OpResult, ProjectEngine,
    };
    use tokio::sync::RwLock;

    struct ExecMockBackend {
        engine: RwLock<ProjectEngine>,
        keyframes: Vec<KeyframeResult>,
        assets: Vec<SavedCodeAsset>,
    }

    impl SolarisBackend for ExecMockBackend {
        fn list_projects(&self) -> BoxFuture<'_, Result<Vec<Value>, AgentError>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn create_project(
            &self,
            _name: String,
            _aspect: Option<AspectPreset>,
            _brand: Option<String>,
            _prompt: Option<String>,
            _template: Option<String>,
        ) -> BoxFuture<'_, Result<Project, AgentError>> {
            Box::pin(async { Err(AgentError::Backend("unused".into())) })
        }
        fn get_project(&self, _id: &str) -> BoxFuture<'_, Result<Project, AgentError>> {
            Box::pin(async { Ok(self.engine.read().await.project().clone()) })
        }
        fn apply_op(
            &self,
            _id: &str,
            cmd: TimelineCommand,
            meta: CommandMeta,
        ) -> BoxFuture<'_, Result<(OpResult, Project), AgentError>> {
            Box::pin(async move {
                let mut g = self.engine.write().await;
                let (res, _) = g
                    .apply(cmd, meta)
                    .map_err(|e| AgentError::ToolError(e.to_string()))?;
                Ok((res, g.project().clone()))
            })
        }
        fn undo(&self, _id: &str) -> BoxFuture<'_, Result<Project, AgentError>> {
            Box::pin(async move {
                let mut g = self.engine.write().await;
                g.undo().map_err(|e| AgentError::ToolError(e.to_string()))?;
                Ok(g.project().clone())
            })
        }
        fn redo(&self, _id: &str) -> BoxFuture<'_, Result<Project, AgentError>> {
            Box::pin(async { Err(AgentError::Backend("unused".into())) })
        }
        fn acquire_track_lock(
            &self,
            _pid: &str,
            _tid: &str,
            _aid: &str,
            _role: AgentRole,
        ) -> BoxFuture<'_, Result<(), AgentError>> {
            Box::pin(async { Ok(()) })
        }
        fn release_track_lock(
            &self,
            _pid: &str,
            _tid: &str,
            _aid: &str,
        ) -> BoxFuture<'_, Result<(), AgentError>> {
            Box::pin(async { Ok(()) })
        }
        fn import_media_paths(
            &self,
            _pid: &str,
            _paths: Vec<(String, Option<String>)>,
        ) -> BoxFuture<'_, Result<Vec<MediaAsset>, AgentError>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn transcribe_media(
            &self,
            _pid: &str,
            _mid: &str,
            _lang: Option<String>,
        ) -> BoxFuture<'_, Result<Vec<TranscriptWord>, AgentError>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn extract_keyframes(
            &self,
            _pid: &str,
            _mid: &str,
            _ts: Vec<f64>,
        ) -> BoxFuture<'_, Result<Vec<KeyframeResult>, AgentError>> {
            let frames = self.keyframes.clone();
            Box::pin(async move { Ok(frames) })
        }
        fn capture_preview_frame(
            &self,
            _pid: &str,
            _ts: f64,
        ) -> BoxFuture<'_, Result<CapturedFrame, AgentError>> {
            Box::pin(async { Err(AgentError::Backend("unused".into())) })
        }
        fn list_brands(&self) -> BoxFuture<'_, Result<Vec<BrandPlaybook>, AgentError>> {
            Box::pin(async { Ok(vec![BrandPlaybook::solaris_default()]) })
        }
        fn get_brand(&self, _bid: &str) -> BoxFuture<'_, Result<BrandPlaybook, AgentError>> {
            Box::pin(async { Ok(BrandPlaybook::solaris_default()) })
        }
        fn list_assets(&self) -> BoxFuture<'_, Result<Vec<SavedCodeAsset>, AgentError>> {
            let assets = self.assets.clone();
            Box::pin(async move { Ok(assets) })
        }
        fn save_asset(
            &self,
            a: SavedCodeAsset,
        ) -> BoxFuture<'_, Result<SavedCodeAsset, AgentError>> {
            Box::pin(async { Ok(a) })
        }
        fn start_export(
            &self,
            _pid: &str,
            _req: ExportRequest,
        ) -> BoxFuture<'_, Result<ExportJobStatus, AgentError>> {
            Box::pin(async { Err(AgentError::Backend("unused".into())) })
        }
        fn emit_cursor_pulse(
            &self,
            _pid: &str,
            _aid: &str,
            _role: AgentRole,
            _anchor: &str,
            _label: &str,
        ) -> BoxFuture<'_, Result<(), AgentError>> {
            Box::pin(async { Ok(()) })
        }
        fn emit_playhead_seek(
            &self,
            _pid: &str,
            _ts: f64,
            _aid: &str,
            _role: AgentRole,
        ) -> BoxFuture<'_, Result<(), AgentError>> {
            Box::pin(async { Ok(()) })
        }
    }

    #[tokio::test]
    async fn project_get_strips_waveform_peaks_and_keyframes_uses_images_vec() {
        let mut proj = Project::new_default("compact_test", AspectPreset::Vertical9x16);
        let pid = proj.id.clone();
        proj.media_pool.push(MediaAsset {
            id: "m1".to_string(),
            name: "talk.mp4".to_string(),
            kind: MediaKind::Video,
            role_hint: "a_roll".to_string(),
            path: "/tmp/talk.mp4".to_string(),
            added_at: proj.created_at,
            intelligence: MediaIntelligenceManifest {
                analyzed: true,
                duration_sec: 4.0,
                waveform_peaks: vec![0.1, 0.5, 0.8, 0.3],
                ..MediaIntelligenceManifest::default()
            },
        });
        let backend = ExecMockBackend {
            engine: RwLock::new(ProjectEngine::new(proj)),
            keyframes: vec![
                KeyframeResult {
                    time_sec: 0.5,
                    png_base64: "AAA".to_string(),
                },
                KeyframeResult {
                    time_sec: 2.0,
                    png_base64: "BBB".to_string(),
                },
            ],
            assets: Vec::new(),
        };

        let p_out = execute_tool(
            &backend,
            "project.get",
            json!({ "project_id": pid }),
            CommandMeta::default(),
        )
        .await
        .unwrap();
        let intel = &p_out.structured["media_pool"][0]["intelligence"];
        assert!(intel.get("waveform_peaks").is_none());
        assert_eq!(intel["waveform_peaks_count"], 4);

        let kf_out = execute_tool(
            &backend,
            "media.extract_keyframes",
            json!({ "project_id": pid, "media_id": "m1" }),
            CommandMeta::default(),
        )
        .await
        .unwrap();
        assert_eq!(kf_out.all_images_base64(), vec!["AAA", "BBB"]);
        assert!(
            kf_out.structured["keyframes"][0]
                .get("png_base64")
                .is_none()
        );
        assert_eq!(kf_out.structured["keyframes"][0]["time_sec"], 0.5);
    }

    #[tokio::test]
    async fn captions_generate_deduplicates_v1_a1_collects_all_assets_and_preserves_on_empty_map() {
        let mut proj = Project::new_default("cap_test", AspectPreset::Vertical9x16);
        let pid = proj.id.clone();
        proj.media_pool.push(MediaAsset {
            id: "m1".to_string(),
            name: "part1.mp4".to_string(),
            kind: MediaKind::Video,
            role_hint: "a_roll".to_string(),
            path: "/tmp/p1.mp4".to_string(),
            added_at: proj.created_at,
            intelligence: MediaIntelligenceManifest {
                analyzed: true,
                duration_sec: 3.0,
                transcript: vec![TranscriptWord {
                    word: "Hello".into(),
                    start_sec: 0.1,
                    end_sec: 0.6,
                }],
                ..MediaIntelligenceManifest::default()
            },
        });
        proj.media_pool.push(MediaAsset {
            id: "m2".to_string(),
            name: "part2.mp4".to_string(),
            kind: MediaKind::Video,
            role_hint: "a_roll".to_string(),
            path: "/tmp/p2.mp4".to_string(),
            added_at: proj.created_at,
            intelligence: MediaIntelligenceManifest {
                analyzed: true,
                duration_sec: 3.0,
                transcript: vec![TranscriptWord {
                    word: "World".into(),
                    start_sec: 0.2,
                    end_sec: 0.8,
                }],
                ..MediaIntelligenceManifest::default()
            },
        });

        let mut engine = ProjectEngine::new(proj);
        let meta = CommandMeta::default();
        // Place m1 identically on BOTH v1 and a1 (must deduplicate!) plus m2 on v1 at 3.0s
        for (tid, cid, mid, start) in [
            ("v1", "c_v1_1", "m1", 0.0),
            ("a1", "c_a1_1", "m1", 0.0),
            ("v1", "c_v1_2", "m2", 3.0),
        ] {
            engine
                .apply(
                    TimelineCommand::PlaceClip {
                        track_id: tid.into(),
                        clip: Clip {
                            id: cid.into(),
                            label: cid.into(),
                            start_sec: start,
                            duration_sec: 2.0,
                            source_in_sec: 0.0,
                            transform: Transform2D::default(),
                            audio: AudioSettings::default(),
                            payload: ClipKind::Media {
                                media_id: mid.into(),
                                ken_burns: false,
                            },
                        },
                    },
                    meta.clone(),
                )
                .unwrap();
        }

        let backend = ExecMockBackend {
            engine: RwLock::new(engine),
            keyframes: Vec::new(),
            assets: Vec::new(),
        };

        let out = execute_tool(
            &backend,
            "captions.generate_from_transcript",
            json!({ "project_id": pid, "words_per_burst": 3 }),
            meta.clone(),
        )
        .await
        .unwrap();
        assert_eq!(out.structured["placed_count"], 2);
        {
            let g = backend.engine.read().await;
            let t1 = g.project().tracks.iter().find(|t| t.id == "t1").unwrap();
            assert_eq!(t1.clips.len(), 2);
            assert_eq!(t1.clips[0].label, "Hello");
            assert_eq!(t1.clips[1].label, "World");
        }

        // Now shift m1 clip's source_in past all words and request captions for m1 only:
        // must fail BEFORE deleting existing T1 captions!
        backend
            .apply_op(
                &pid,
                TimelineCommand::TrimClip {
                    track_id: "v1".into(),
                    clip_id: "c_v1_1".into(),
                    new_start_sec: None,
                    new_duration_sec: None,
                    new_source_in_sec: Some(2.0),
                },
                meta.clone(),
            )
            .await
            .unwrap();
        backend
            .apply_op(
                &pid,
                TimelineCommand::TrimClip {
                    track_id: "a1".into(),
                    clip_id: "c_a1_1".into(),
                    new_start_sec: None,
                    new_duration_sec: None,
                    new_source_in_sec: Some(2.0),
                },
                meta.clone(),
            )
            .await
            .unwrap();
        let err = execute_tool(
            &backend,
            "captions.generate_from_transcript",
            json!({ "project_id": pid, "media_id": "m1" }),
            meta,
        )
        .await;
        assert!(err.is_err());
        let g = backend.engine.read().await;
        let t1 = g.project().tracks.iter().find(|t| t.id == "t1").unwrap();
        assert_eq!(t1.clips.len(), 2, "existing T1 captions must be preserved");
    }

    #[tokio::test]
    async fn captions_place_script_preserves_segment_and_clause_boundaries_without_orphans() {
        let project = Project::new_default("script_caps", AspectPreset::Vertical9x16);
        let pid = project.id.clone();
        let backend = ExecMockBackend {
            engine: RwLock::new(ProjectEngine::new(project)),
            keyframes: Vec::new(),
            assets: Vec::new(),
        };
        let meta = CommandMeta {
            agent_id: "caps-test".into(),
            role: AgentRole::Caps,
            reason: Some("caps-test".into()),
        };

        let out = execute_tool(
            &backend,
            "captions.place_script",
            json!({
                "project_id": pid,
                "words_per_burst": 3,
                "y": 0.24,
                "segments": [
                    { "text": "Skills make agents capable.", "start_sec": 0.0, "duration_sec": 3.0 },
                    { "text": "Not prompts. Reusable abilities.", "start_sec": 3.0, "duration_sec": 3.5 },
                    { "text": "Instructions, tools, examples.", "start_sec": 6.5, "duration_sec": 3.5 }
                ]
            }),
            meta,
        )
        .await
        .unwrap();
        assert_eq!(out.structured["placed_count"], 7);
        assert_eq!(out.structured["truncated_segments"], 0);

        let g = backend.engine.read().await;
        let t1 = g.project().tracks.iter().find(|t| t.id == "t1").unwrap();
        let labels: Vec<&str> = t1.clips.iter().map(|c| c.label.as_str()).collect();
        // 4-word sentence balances 2+2 (no 3+1 orphan), and periods/commas/segments never bleed together
        assert_eq!(
            labels,
            vec![
                "Skills make",
                "agents capable.",
                "Not prompts.",
                "Reusable abilities.",
                "Instructions,",
                "tools,",
                "examples."
            ]
        );
    }

    #[tokio::test]
    async fn captions_place_script_handles_overlaps_dense_words_validation_and_truncation() {
        let project = Project::new_default("script_overlap_test", AspectPreset::Vertical9x16);
        let pid = project.id.clone();
        let backend = ExecMockBackend {
            engine: RwLock::new(ProjectEngine::new(project)),
            keyframes: Vec::new(),
            assets: Vec::new(),
        };
        let meta = CommandMeta {
            agent_id: "caps-test".into(),
            role: AgentRole::Caps,
            reason: Some("caps-test".into()),
        };

        // Overlapping segments + dense 10-word segment in 0.4s must produce non-overlapping clips
        // with every word strictly clamped inside [clip.start_sec, clip.start_sec + clip.duration_sec].
        let out = execute_tool(
            &backend,
            "captions.place_script",
            json!({
                "project_id": pid,
                "words_per_burst": 3,
                "segments": [
                    { "text": "Second overlapping segment here", "start_sec": 1.0, "duration_sec": 2.0 },
                    { "text": "First long segment overlaps second", "start_sec": 0.0, "duration_sec": 3.0 },
                    { "text": "one two three four five six seven eight nine ten", "start_sec": 3.0, "duration_sec": 0.4 }
                ]
            }),
            meta.clone(),
        )
        .await
        .unwrap();
        assert!(out.structured["placed_count"].as_u64().unwrap() >= 5);

        let initial_clip_count = {
            let g = backend.engine.read().await;
            let t1 = g.project().tracks.iter().find(|t| t.id == "t1").unwrap();
            for pair in t1.clips.windows(2) {
                let prev_end = pair[0].start_sec + pair[0].duration_sec;
                assert!(
                    prev_end <= pair[1].start_sec + 1e-6,
                    "T1 clips must not overlap: prev_end={prev_end}, next_start={}",
                    pair[1].start_sec
                );
            }
            for clip in &t1.clips {
                let clip_end = clip.start_sec + clip.duration_sec;
                if let ClipKind::Caption { caption } = &clip.payload {
                    for w in &caption.words {
                        assert!(
                            w.start_sec >= clip.start_sec - 1e-9 && w.end_sec <= clip_end + 1e-9,
                            "word {:?} [{}, {}] escaped clip [{}, {}]",
                            w.text,
                            w.start_sec,
                            w.end_sec,
                            clip.start_sec,
                            clip_end
                        );
                        assert!(w.start_sec <= w.end_sec + 1e-9);
                    }
                }
            }
            t1.clips.len()
        };

        // String "false" for replace_existing must be rejected (and existing T1 clips preserved)
        let bad_replace = execute_tool(
            &backend,
            "captions.place_script",
            json!({
                "project_id": pid,
                "replace_existing": "false",
                "segments": [
                    { "text": "Should fail", "start_sec": 0.0, "duration_sec": 1.0 }
                ]
            }),
            meta.clone(),
        )
        .await;
        assert!(matches!(bad_replace, Err(AgentError::ToolError(_))));
        {
            let g = backend.engine.read().await;
            let t1 = g.project().tracks.iter().find(|t| t.id == "t1").unwrap();
            assert_eq!(t1.clips.len(), initial_clip_count);
        }

        // Invalid segment later in array must fail BEFORE deleting existing T1 clips
        let bad_seg = execute_tool(
            &backend,
            "captions.place_script",
            json!({
                "project_id": pid,
                "replace_existing": true,
                "segments": [
                    { "text": "Valid first", "start_sec": 0.0, "duration_sec": 1.0 },
                    { "text": "Bad second", "start_sec": 1.0, "duration_sec": 0.01 }
                ]
            }),
            meta.clone(),
        )
        .await;
        assert!(matches!(bad_seg, Err(AgentError::ToolError(_))));
        {
            let g = backend.engine.read().await;
            let t1 = g.project().tracks.iter().find(|t| t.id == "t1").unwrap();
            assert_eq!(t1.clips.len(), initial_clip_count);
        }

        // More than 64 segments must report truncated_segments instead of silently dropping
        let many_segments: Vec<Value> = (0..67)
            .map(|i| {
                json!({
                    "text": format!("Beat {i}"),
                    "start_sec": (i as f64) * 0.5,
                    "duration_sec": 0.45,
                })
            })
            .collect();
        let trunc_out = execute_tool(
            &backend,
            "captions.place_script",
            json!({
                "project_id": pid,
                "segments": many_segments,
            }),
            meta,
        )
        .await
        .unwrap();
        assert_eq!(trunc_out.structured["truncated_segments"], 3);
    }

    #[tokio::test]
    async fn gfx_place_treats_null_props_as_empty_object_and_rejects_non_object() {
        let project = Project::new_default("gfx_props_test", AspectPreset::Vertical9x16);
        let pid = project.id.clone();
        let asset = SavedCodeAsset {
            id: "card-1".to_string(),
            name: "Card".to_string(),
            category: "cards".to_string(),
            tags: vec![],
            props_schema: json!({ "type": "object", "properties": {} }),
            default_props: json!({ "headline": "Default Title", "accent": "#F05A00" }),
            html_source:
                r#"<div data-start="0" data-duration="2" style="color:var(--brand-ink)">Card</div>"#
                    .to_string(),
            default_duration_sec: 2.0,
            created_at: project.created_at,
        };
        let backend = ExecMockBackend {
            engine: RwLock::new(ProjectEngine::new(project)),
            keyframes: Vec::new(),
            assets: vec![asset],
        };
        let meta = CommandMeta::default();

        // Explicit `"props": null` must preserve `default_props` rather than replacing with null
        execute_tool(
            &backend,
            "gfx.place",
            json!({
                "project_id": pid,
                "clip_id": "g1-null-props",
                "label": "Null Props Card",
                "start_sec": 0.0,
                "duration_sec": 2.0,
                "asset_id": "card-1",
                "props": null
            }),
            meta.clone(),
        )
        .await
        .unwrap();

        {
            let g = backend.engine.read().await;
            let g1 = g.project().tracks.iter().find(|t| t.id == "g1").unwrap();
            let clip = g1.clips.iter().find(|c| c.id == "g1-null-props").unwrap();
            match &clip.payload {
                ClipKind::Gfx { gfx } => {
                    assert_eq!(gfx.props["headline"], "Default Title");
                    assert_eq!(gfx.props["accent"], "#F05A00");
                }
                other => panic!("expected Gfx clip, got {other:?}"),
            }
        }

        // Alias keys (`html` / `html_source`) are accepted when `inline_html` is omitted
        execute_tool(
            &backend,
            "gfx.place",
            json!({
                "project_id": pid,
                "clip_id": "g1-html-alias",
                "label": "Alias HTML Card",
                "start_sec": 2.0,
                "duration_sec": 2.0,
                "html_source": "<div class=\"clip\" data-start=\"0\" data-duration=\"2\" style=\"color:var(--brand-ink)\">Alias</div>"
            }),
            meta.clone(),
        )
        .await
        .unwrap();

        // Non-object props (string/array) must fail with ToolError
        let err = execute_tool(
            &backend,
            "gfx.place",
            json!({
                "project_id": pid,
                "clip_id": "g1-bad-props",
                "label": "Bad Props Card",
                "start_sec": 4.0,
                "duration_sec": 2.0,
                "asset_id": "card-1",
                "props": "not-an-object"
            }),
            meta,
        )
        .await;
        assert!(matches!(err, Err(AgentError::ToolError(_))));
    }

    #[test]
    fn caption_punctuation_and_tokenization_support_abbreviations_cjk_and_rtl() {
        // Common abbreviations & single-digit ordinals should NOT trigger a clause break
        assert!(!ends_with_clause_punctuation("Dr."));
        assert!(!ends_with_clause_punctuation("Mr."));
        assert!(!ends_with_clause_punctuation("e.g."));
        assert!(!ends_with_clause_punctuation("vs."));
        assert!(!ends_with_clause_punctuation("1."));

        // CJK, RTL, and closing quotes/brackets SHOULD trigger a clause break
        assert!(ends_with_clause_punctuation("完成。"));
        assert!(ends_with_clause_punctuation("「太棒了！」"));
        assert!(ends_with_clause_punctuation("（第一步，）"));
        assert!(ends_with_clause_punctuation("مرحبا،"));
        assert!(ends_with_clause_punctuation("كيف؟"));
        assert!(ends_with_clause_punctuation("done!»"));

        // Whitespace-free CJK strings > 6 chars split into safe-zone-friendly chunks
        let cjk_tokens = tokenize_caption_text("这是一个没有空格的长中文句子，用来测试自动分段。");
        assert!(cjk_tokens.len() >= 5, "got {:?}", cjk_tokens);
        assert!(cjk_tokens.iter().all(|t| t.chars().count() <= 6));
        assert!(cjk_tokens.iter().any(|t| t.ends_with('，')));
        assert!(cjk_tokens.last().unwrap().ends_with('。'));

        // Abbreviation stays attached to the following word in the same burst
        let words: Vec<TranscriptWord> = ["Dr.", "Smith", "explained", "e.g.", "fast", "cuts."]
            .iter()
            .enumerate()
            .map(|(i, w)| TranscriptWord {
                word: (*w).to_string(),
                start_sec: i as f64 * 0.4,
                end_sec: (i as f64 + 0.9) * 0.4,
            })
            .collect();
        let bursts = split_caption_bursts(&words, 3, 0.6);
        let texts: Vec<String> = bursts
            .iter()
            .map(|b| {
                b.iter()
                    .map(|w| w.word.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        assert_eq!(texts, vec!["Dr. Smith explained", "e.g. fast cuts."]);
    }

    fn caption_test_clip(start_sec: f64, duration_sec: f64) -> Clip {
        Clip {
            id: "cap-test".to_string(),
            label: "Caption".to_string(),
            start_sec,
            duration_sec,
            source_in_sec: 0.0,
            transform: Transform2D::default(),
            audio: AudioSettings::default(),
            payload: ClipKind::Caption {
                caption: CaptionBlock {
                    words: Vec::new(),
                    preset: CaptionPreset::Minimal,
                    font_family: "Sora".to_string(),
                    highlight_color: "#F7B733".to_string(),
                },
            },
        }
    }

    #[test]
    fn retime_caption_words_keeps_transcript_timing_when_word_count_matches() {
        let original = vec![
            CaptionWord {
                text: "hello".to_string(),
                start_sec: 0.5,
                end_sec: 1.0,
            },
            CaptionWord {
                text: "world".to_string(),
                start_sec: 1.0,
                end_sec: 1.8,
            },
        ];
        let words = retime_caption_words(&original, "hi there", &caption_test_clip(0.5, 2.0));
        assert_eq!(words[0].text, "hi");
        assert_eq!((words[0].start_sec, words[0].end_sec), (0.5, 1.0));
        assert_eq!(words[1].text, "there");
        assert_eq!((words[1].start_sec, words[1].end_sec), (1.0, 1.8));
    }

    #[test]
    fn retime_caption_words_spreads_changed_word_count_across_spoken_span() {
        let original = vec![
            CaptionWord {
                text: "a".to_string(),
                start_sec: 2.0,
                end_sec: 3.0,
            },
            CaptionWord {
                text: "b".to_string(),
                start_sec: 3.0,
                end_sec: 4.0,
            },
        ];
        let words = retime_caption_words(
            &original,
            "one two three four",
            &caption_test_clip(2.0, 2.0),
        );
        assert_eq!(words.len(), 4);
        assert!((words[0].start_sec - 2.0).abs() < 1e-9);
        assert!((words[3].end_sec - 4.0).abs() < 1e-9);
    }

    #[test]
    fn retime_caption_words_never_drops_words_from_long_text() {
        let text = (0..80)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let words = retime_caption_words(&[], &text, &caption_test_clip(0.0, 10.0));
        assert_eq!(words.len(), 80);
        assert_eq!(words.last().unwrap().text, "w79");
    }

    #[tokio::test]
    async fn gfx_update_clip_updates_in_place_keeps_transform_deep_merges_lints_and_is_one_undo() {
        let project = Project::new_default("gfx_update_test", AspectPreset::Vertical9x16);
        let pid = project.id.clone();
        let mut engine = ProjectEngine::new(project);
        let meta = CommandMeta::default();
        let original_html =
            r#"<div class="clip" data-start="0" data-duration="3.0">{{title}}</div>"#;
        engine
            .apply(
                TimelineCommand::PlaceGfx {
                    track_id: "g1".into(),
                    clip_id: "gfx-1".into(),
                    label: "Hero Card".into(),
                    start_sec: 1.0,
                    duration_sec: 3.0,
                    gfx: GfxInstance {
                        asset_id: None,
                        props: json!({ "title": "Old", "theme": { "bg": "#080404", "ink": "#F7F5EF" } }),
                        inline_html: Some(original_html.into()),
                    },
                },
                meta.clone(),
            )
            .unwrap();
        let custom_transform = Transform2D {
            x: 0.12,
            y: -0.18,
            scale: 1.15,
            rotation_deg: 4.0,
            opacity: 0.9,
            smart_reframe: None,
        };
        engine
            .apply(
                TimelineCommand::UpdateClipTransform {
                    track_id: "g1".into(),
                    clip_id: "gfx-1".into(),
                    transform: custom_transform.clone(),
                },
                meta.clone(),
            )
            .unwrap();
        // Also place a non-GFX clip on g1 via PlaceClip? No, place a caption clip on t1 to test unknown id on g1.
        let backend = ExecMockBackend {
            engine: RwLock::new(engine),
            keyframes: Vec::new(),
            assets: Vec::new(),
        };

        // Rejects unknown clip id
        assert!(
            execute_tool(
                &backend,
                "gfx.update_clip",
                json!({ "project_id": pid, "clip_id": "missing", "label": "X" }),
                meta.clone(),
            )
            .await
            .is_err()
        );

        // Rejects unsafe HTML via linter
        assert!(
            execute_tool(
                &backend,
                "gfx.update_clip",
                json!({
                    "project_id": pid,
                    "clip_id": "gfx-1",
                    "inline_html": "<script>fetch('https://evil.example')</script>"
                }),
                meta.clone(),
            )
            .await
            .is_err()
        );

        let rev_before = backend.get_project(&pid).await.unwrap().revision;
        execute_tool(
            &backend,
            "gfx.update_clip",
            json!({
                "project_id": pid,
                "clip_id": "gfx-1",
                "label": "Updated Hero",
                "props": { "theme": { "bg": "#1A0E0E" }, "badge": "NEW" }
            }),
            meta.clone(),
        )
        .await
        .unwrap();

        let after = backend.get_project(&pid).await.unwrap();
        assert_eq!(after.revision, rev_before + 1);
        let g1 = after.tracks.iter().find(|t| t.id == "g1").unwrap();
        assert_eq!(g1.clips.len(), 1);
        let clip = &g1.clips[0];
        assert_eq!(clip.id, "gfx-1");
        assert_eq!(clip.label, "Updated Hero");
        assert!((clip.transform.x - 0.12).abs() < 1e-9);
        assert!((clip.transform.y - -0.18).abs() < 1e-9);
        assert!((clip.transform.scale - 1.15).abs() < 1e-9);
        match &clip.payload {
            ClipKind::Gfx { gfx } => {
                assert_eq!(gfx.props["title"], "Old");
                assert_eq!(gfx.props["theme"]["bg"], "#1A0E0E");
                assert_eq!(gfx.props["theme"]["ink"], "#F7F5EF");
                assert_eq!(gfx.props["badge"], "NEW");
            }
            other => panic!("expected Gfx clip, got {other:?}"),
        }

        // Single undo reverts the entire gfx.update_clip in one step
        let undone = backend.undo(&pid).await.unwrap();
        let g1_undone = undone.tracks.iter().find(|t| t.id == "g1").unwrap();
        assert_eq!(g1_undone.clips[0].label, "Hero Card");
    }

    #[tokio::test]
    async fn captions_update_clip_validates_kind_presets_and_word_timings() {
        let project = Project::new_default("cap_update_test", AspectPreset::Vertical9x16);
        let pid = project.id.clone();
        let mut engine = ProjectEngine::new(project);
        let meta = CommandMeta::default();
        let cap_clip = Clip {
            id: "cap-1".into(),
            label: "Hello world".into(),
            start_sec: 1.0,
            duration_sec: 2.0,
            source_in_sec: 0.0,
            transform: Transform2D::default(),
            audio: AudioSettings::default(),
            payload: ClipKind::Caption {
                caption: CaptionBlock {
                    words: vec![
                        CaptionWord {
                            text: "Hello".into(),
                            start_sec: 1.0,
                            end_sec: 1.7,
                        },
                        CaptionWord {
                            text: "world".into(),
                            start_sec: 1.7,
                            end_sec: 2.8,
                        },
                    ],
                    preset: CaptionPreset::Minimal,
                    font_family: "Sora".into(),
                    highlight_color: "#F7B733".into(),
                },
            },
        };
        engine
            .apply(
                TimelineCommand::PlaceClip {
                    track_id: "t1".into(),
                    clip: cap_clip,
                },
                meta.clone(),
            )
            .unwrap();
        let backend = ExecMockBackend {
            engine: RwLock::new(engine),
            keyframes: Vec::new(),
            assets: Vec::new(),
        };

        // Rejects unknown / non-caption clip
        assert!(
            execute_tool(
                &backend,
                "captions.update_clip",
                json!({ "project_id": pid, "clip_id": "missing", "preset": "karaoke" }),
                meta.clone(),
            )
            .await
            .is_err()
        );

        // All four presets parse cleanly, and same word count preserves timings
        for preset in ["neon_glow", "mono_caps", "karaoke", "minimal"] {
            execute_tool(
                &backend,
                "captions.update_clip",
                json!({
                    "project_id": pid,
                    "clip_id": "cap-1",
                    "preset": preset,
                    "text": "Hi there"
                }),
                meta.clone(),
            )
            .await
            .unwrap();
        }
        let after_same = backend.get_project(&pid).await.unwrap();
        let t1 = after_same.tracks.iter().find(|t| t.id == "t1").unwrap();
        if let ClipKind::Caption { caption } = &t1.clips[0].payload {
            assert_eq!(caption.preset, CaptionPreset::Minimal);
            assert_eq!(caption.words.len(), 2);
            assert_eq!(
                (caption.words[0].start_sec, caption.words[0].end_sec),
                (1.0, 1.7)
            );
            assert_eq!(
                (caption.words[1].start_sec, caption.words[1].end_sec),
                (1.7, 2.8)
            );
        } else {
            panic!("expected caption clip");
        }

        // Different word count spans the original spoken range [1.0, 2.8]
        execute_tool(
            &backend,
            "captions.update_clip",
            json!({
                "project_id": pid,
                "clip_id": "cap-1",
                "text": "One two three four"
            }),
            meta,
        )
        .await
        .unwrap();
        let after_diff = backend.get_project(&pid).await.unwrap();
        let t1_diff = after_diff.tracks.iter().find(|t| t.id == "t1").unwrap();
        if let ClipKind::Caption { caption } = &t1_diff.clips[0].payload {
            assert_eq!(caption.words.len(), 4);
            assert!((caption.words[0].start_sec - 1.0).abs() < 1e-9);
            assert!((caption.words[3].end_sec - 2.8).abs() < 1e-9);
        } else {
            panic!("expected caption clip");
        }
    }

    #[tokio::test]
    async fn batch_atomicity_through_backend_apply_op_rolls_back_on_error() {
        let project = Project::new_default("batch_atomic_test", AspectPreset::Vertical9x16);
        let pid = project.id.clone();
        let backend = ExecMockBackend {
            engine: RwLock::new(ProjectEngine::new(project)),
            keyframes: Vec::new(),
            assets: Vec::new(),
        };
        let rev_before = backend.get_project(&pid).await.unwrap().revision;
        let err = backend
            .apply_op(
                &pid,
                TimelineCommand::Batch {
                    commands: vec![
                        TimelineCommand::PlaceGfx {
                            track_id: "g1".into(),
                            clip_id: "ok-clip".into(),
                            label: "Valid".into(),
                            start_sec: 0.0,
                            duration_sec: 2.0,
                            gfx: GfxInstance {
                                asset_id: None,
                                props: json!({}),
                                inline_html: Some(
                                    "<div data-start='0' data-duration='2'>Ok</div>".into(),
                                ),
                            },
                        },
                        TimelineCommand::DeleteClip {
                            track_id: "g1".into(),
                            clip_id: "does-not-exist".into(),
                        },
                    ],
                    summary: Some("atomic rollback test".into()),
                },
                CommandMeta::default(),
            )
            .await;
        assert!(err.is_err());
        let after = backend.get_project(&pid).await.unwrap();
        assert_eq!(after.revision, rev_before);
        let g1 = after.tracks.iter().find(|t| t.id == "g1").unwrap();
        assert!(
            g1.clips.is_empty(),
            "rolled-back batch must leave g1 untouched"
        );
    }
}
