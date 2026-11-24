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

fn split_caption_bursts(
    words: &[TranscriptWord],
    words_per_burst: usize,
    silence_gap_sec: f64,
) -> Vec<Vec<&TranscriptWord>> {
    let mut bursts: Vec<Vec<&TranscriptWord>> = Vec::new();
    let mut current: Vec<&TranscriptWord> = Vec::new();
    for w in words {
        let gap_exceeded = current
            .last()
            .is_some_and(|prev| w.start_sec - prev.end_sec > silence_gap_sec);
        if !current.is_empty() && (current.len() >= words_per_burst || gap_exceeded) {
            bursts.push(std::mem::take(&mut current));
        }
        current.push(w);
    }
    if !current.is_empty() {
        bursts.push(current);
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

            let font_family = brand
                .as_ref()
                .map(|b| b.tokens.font_display.clone())
                .unwrap_or_else(|| "Sora".to_string());
            let highlight_color = brand
                .as_ref()
                .map(|b| b.tokens.accent_secondary.clone())
                .unwrap_or_else(|| "#F7B733".to_string());

            let bursts = split_caption_bursts(&timeline_words, words_per_burst, 0.6);
            let mut placed = 0usize;
            for (idx, chunk) in bursts.iter().enumerate() {
                let Some(first) = chunk.first() else { continue };
                let Some(last) = chunk.last() else { continue };
                let start_sec = first.start_sec;
                let mut duration_sec = (last.end_sec - first.start_sec).max(0.25);
                if let Some(next_start) = bursts
                    .get(idx + 1)
                    .and_then(|next| next.first())
                    .map(|w| w.start_sec)
                {
                    let max_no_overlap = (next_start - start_sec - 0.02).max(0.05);
                    duration_sec = duration_sec.min(max_no_overlap);
                }
                let label = chunk
                    .iter()
                    .map(|w| w.word.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                let words = chunk
                    .iter()
                    .map(|w| CaptionWord {
                        text: w.word.clone(),
                        start_sec: w.start_sec,
                        end_sec: w.end_sec,
                    })
                    .collect();
                let clip = Clip {
                    id: format!("cap-{idx}-{}", uuid::Uuid::new_v4().simple()),
                    label,
                    start_sec,
                    duration_sec,
                    source_in_sec: 0.0,
                    transform: Transform2D {
                        y: 0.20,
                        ..Transform2D::default()
                    },
                    audio: AudioSettings::default(),
                    payload: ClipKind::Caption {
                        caption: CaptionBlock {
                            words,
                            preset,
                            font_family: font_family.clone(),
                            highlight_color: highlight_color.clone(),
                        },
                    },
                };
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
            let mut props = args.get("props").cloned().unwrap_or_else(|| json!({}));
            let mut inline_html = args
                .get("inline_html")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);

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
            Box::pin(async { Err(AgentError::Backend("unused".into())) })
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
            Box::pin(async { Ok(vec![]) })
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
}
