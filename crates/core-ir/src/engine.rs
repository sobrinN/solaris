use std::collections::HashMap;
use std::fs;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::commands::{AgentRole, CommandMeta, EngineEvent, OpResult, TimelineCommand};
use crate::error::CoreError;
use crate::model::{
    AudioSettings, CaptionBlock, Clip, ClipKind, ColorGrade, GfxInstance,
    MediaIntelligenceManifest, OpLogEntry, Project, SmartReframeBox, Transform2D,
};
use crate::registry::MAX_GFX_HTML_BYTES;

const TRACK_LOCK_TTL_SECS: i64 = 300;

fn ensure_finite(val: f64, label: &str) -> Result<(), CoreError> {
    if !val.is_finite() {
        return Err(CoreError::InvalidOperation(format!(
            "{label} must be a finite number"
        )));
    }
    Ok(())
}

fn ensure_finite_span(
    start_sec: f64,
    duration_sec: f64,
    source_in_sec: f64,
) -> Result<(), CoreError> {
    ensure_finite(start_sec, "start_sec")?;
    ensure_finite(duration_sec, "duration_sec")?;
    ensure_finite(source_in_sec, "source_in_sec")?;
    ensure_finite(start_sec + duration_sec, "start_sec + duration_sec")?;
    ensure_finite(source_in_sec + duration_sec, "source_in_sec + duration_sec")?;
    Ok(())
}

fn validate_gfx_instance(gfx: &GfxInstance) -> Result<(), CoreError> {
    if let Some(html) = &gfx.inline_html
        && html.len() > MAX_GFX_HTML_BYTES
    {
        return Err(CoreError::InvalidOperation(format!(
            "gfx inline_html exceeds maximum size of {MAX_GFX_HTML_BYTES} bytes"
        )));
    }
    Ok(())
}

fn validate_smart_reframe(rf: &SmartReframeBox, prefix: &str) -> Result<(), CoreError> {
    ensure_finite(rf.x, &format!("{prefix}.x"))?;
    ensure_finite(rf.y, &format!("{prefix}.y"))?;
    ensure_finite(rf.width, &format!("{prefix}.width"))?;
    ensure_finite(rf.height, &format!("{prefix}.height"))?;
    if rf.x < 0.0 || rf.y < 0.0 || rf.width <= 0.0 || rf.height <= 0.0 {
        return Err(CoreError::InvalidOperation(format!(
            "{prefix} requires x >= 0, y >= 0, width > 0, height > 0"
        )));
    }
    Ok(())
}

fn validate_transform(t: &Transform2D) -> Result<(), CoreError> {
    ensure_finite(t.x, "transform.x")?;
    ensure_finite(t.y, "transform.y")?;
    ensure_finite(t.scale, "transform.scale")?;
    ensure_finite(t.rotation_deg, "transform.rotation_deg")?;
    ensure_finite(t.opacity, "transform.opacity")?;
    if t.scale <= 0.0 {
        return Err(CoreError::InvalidOperation(
            "transform.scale must be > 0".to_string(),
        ));
    }
    if !(0.0..=1.0).contains(&t.opacity) {
        return Err(CoreError::InvalidOperation(
            "transform.opacity must be within [0.0, 1.0]".to_string(),
        ));
    }
    if let Some(rf) = &t.smart_reframe {
        validate_smart_reframe(rf, "transform.smart_reframe")?;
    }
    Ok(())
}

fn validate_audio_settings(audio: &AudioSettings) -> Result<(), CoreError> {
    ensure_finite(audio.gain_db, "audio.gain_db")?;
    ensure_finite(audio.duck_target_db, "audio.duck_target_db")?;
    ensure_finite(audio.fade_in_sec, "audio.fade_in_sec")?;
    ensure_finite(audio.fade_out_sec, "audio.fade_out_sec")?;
    if audio.fade_in_sec < 0.0 || audio.fade_out_sec < 0.0 {
        return Err(CoreError::InvalidOperation(
            "audio fade_in_sec and fade_out_sec must be >= 0".to_string(),
        ));
    }
    Ok(())
}

fn validate_color_grade(grade: &ColorGrade) -> Result<(), CoreError> {
    for (val, label) in [
        (grade.lift.r, "lift.r"),
        (grade.lift.g, "lift.g"),
        (grade.lift.b, "lift.b"),
        (grade.gamma.r, "gamma.r"),
        (grade.gamma.g, "gamma.g"),
        (grade.gamma.b, "gamma.b"),
        (grade.gain.r, "gain.r"),
        (grade.gain.g, "gain.g"),
        (grade.gain.b, "gain.b"),
        (grade.temp, "temp"),
        (grade.tint, "tint"),
        (grade.contrast, "contrast"),
        (grade.saturation, "saturation"),
        (grade.exposure, "exposure"),
        (grade.lut_intensity, "lut_intensity"),
    ] {
        ensure_finite(val, label)?;
    }
    if grade.contrast < 0.0 || grade.saturation < 0.0 {
        return Err(CoreError::InvalidOperation(
            "grade contrast and saturation must be >= 0".to_string(),
        ));
    }
    if !(0.0..=1.0).contains(&grade.lut_intensity) {
        return Err(CoreError::InvalidOperation(
            "grade lut_intensity must be within [0.0, 1.0]".to_string(),
        ));
    }
    Ok(())
}

fn validate_caption_block(caption: &CaptionBlock) -> Result<(), CoreError> {
    for (idx, word) in caption.words.iter().enumerate() {
        ensure_finite(word.start_sec, &format!("caption.words[{idx}].start_sec"))?;
        ensure_finite(word.end_sec, &format!("caption.words[{idx}].end_sec"))?;
        if word.start_sec < 0.0 || word.end_sec < word.start_sec {
            return Err(CoreError::InvalidOperation(format!(
                "caption.words[{idx}] requires 0 <= start_sec <= end_sec"
            )));
        }
    }
    Ok(())
}

fn validate_intelligence(intel: &MediaIntelligenceManifest) -> Result<(), CoreError> {
    ensure_finite(intel.duration_sec, "intelligence.duration_sec")?;
    if intel.duration_sec < 0.0 {
        return Err(CoreError::InvalidOperation(
            "intelligence.duration_sec must be >= 0".to_string(),
        ));
    }
    if let Some(fps) = intel.fps {
        ensure_finite(fps, "intelligence.fps")?;
        if fps <= 0.0 {
            return Err(CoreError::InvalidOperation(
                "intelligence.fps must be > 0".to_string(),
            ));
        }
    }
    if let Some(lufs) = intel.integrated_lufs {
        ensure_finite(lufs, "intelligence.integrated_lufs")?;
    }
    if intel.waveform_peaks.iter().any(|p| !p.is_finite()) {
        return Err(CoreError::InvalidOperation(
            "intelligence.waveform_peaks must contain finite numbers".to_string(),
        ));
    }
    for (idx, s) in intel.silences.iter().enumerate() {
        ensure_finite(
            s.start_sec,
            &format!("intelligence.silences[{idx}].start_sec"),
        )?;
        ensure_finite(s.end_sec, &format!("intelligence.silences[{idx}].end_sec"))?;
        if s.start_sec < 0.0 || s.end_sec < s.start_sec {
            return Err(CoreError::InvalidOperation(format!(
                "intelligence.silences[{idx}] requires 0 <= start_sec <= end_sec"
            )));
        }
    }
    for (idx, &b) in intel.beats.iter().enumerate() {
        ensure_finite(b, &format!("intelligence.beats[{idx}]"))?;
        if b < 0.0 {
            return Err(CoreError::InvalidOperation(format!(
                "intelligence.beats[{idx}] must be >= 0"
            )));
        }
    }
    for (idx, &c) in intel.scene_cuts.iter().enumerate() {
        ensure_finite(c, &format!("intelligence.scene_cuts[{idx}]"))?;
        if c < 0.0 {
            return Err(CoreError::InvalidOperation(format!(
                "intelligence.scene_cuts[{idx}] must be >= 0"
            )));
        }
    }
    if let Some(rf) = &intel.default_reframe {
        validate_smart_reframe(rf, "intelligence.default_reframe")?;
    }
    for (idx, w) in intel.transcript.iter().enumerate() {
        ensure_finite(
            w.start_sec,
            &format!("intelligence.transcript[{idx}].start_sec"),
        )?;
        ensure_finite(
            w.end_sec,
            &format!("intelligence.transcript[{idx}].end_sec"),
        )?;
        if w.start_sec < 0.0 || w.end_sec < w.start_sec {
            return Err(CoreError::InvalidOperation(format!(
                "intelligence.transcript[{idx}] requires 0 <= start_sec <= end_sec"
            )));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackLock {
    pub holder_agent_id: String,
    pub role: AgentRole,
    pub acquired_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct HistoryEntry {
    snapshot: Project,
    summary: String,
}

#[derive(Debug, Clone)]
pub struct ProjectEngine {
    project: Project,
    undo_stack: Vec<HistoryEntry>,
    redo_stack: Vec<HistoryEntry>,
    track_locks: HashMap<String, TrackLock>,
}

impl ProjectEngine {
    pub fn new(project: Project) -> Self {
        Self {
            project,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            track_locks: HashMap::new(),
        }
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn track_locks(&self) -> &HashMap<String, TrackLock> {
        &self.track_locks
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn update_metadata(
        &mut self,
        name: Option<String>,
        prompt: Option<String>,
    ) -> Result<EngineEvent, CoreError> {
        if let Some(raw_name) = name {
            let trimmed = raw_name.trim();
            if trimmed.is_empty() {
                return Err(CoreError::InvalidOperation(
                    "project name cannot be empty".to_string(),
                ));
            }
            if trimmed.chars().count() > 160 {
                return Err(CoreError::InvalidOperation(
                    "project name exceeds 160 characters".to_string(),
                ));
            }
            self.project.name = trimmed.to_string();
        }
        if let Some(raw_prompt) = prompt {
            self.project.prompt = raw_prompt;
        }
        self.project.revision = self.project.revision.saturating_add(1);
        self.project.updated_at = Utc::now();
        Ok(EngineEvent::ProjectReset {
            project_id: self.project.id.clone(),
            revision: self.project.revision,
            reason: "metadata_updated".to_string(),
            project: Box::new(self.project.clone()),
        })
    }

    fn prune_expired_locks(&mut self) {
        let now = Utc::now();
        self.track_locks
            .retain(|_, lock| (now - lock.acquired_at).num_seconds() < TRACK_LOCK_TTL_SECS);
    }

    pub fn acquire_track_lock(
        &mut self,
        track_id: &str,
        agent_id: impl Into<String>,
        role: AgentRole,
    ) -> Result<EngineEvent, CoreError> {
        self.prune_expired_locks();
        if !self.project.tracks.iter().any(|t| t.id == track_id) {
            return Err(CoreError::TrackNotFound(track_id.to_string()));
        }
        let agent_id = agent_id.into();
        if let Some(existing) = self.track_locks.get(track_id)
            && existing.holder_agent_id != agent_id
        {
            return Err(CoreError::TrackLocked {
                track_id: track_id.to_string(),
                holder: existing.holder_agent_id.clone(),
            });
        }
        self.track_locks.insert(
            track_id.to_string(),
            TrackLock {
                holder_agent_id: agent_id.clone(),
                role,
                acquired_at: Utc::now(),
            },
        );
        Ok(EngineEvent::TrackLockChanged {
            project_id: self.project.id.clone(),
            track_id: track_id.to_string(),
            holder_agent_id: Some(agent_id),
            role: Some(role),
        })
    }

    pub fn release_track_lock(
        &mut self,
        track_id: &str,
        agent_id: &str,
    ) -> Result<EngineEvent, CoreError> {
        self.prune_expired_locks();
        if let Some(existing) = self.track_locks.get(track_id)
            && existing.holder_agent_id != agent_id
        {
            return Err(CoreError::TrackLocked {
                track_id: track_id.to_string(),
                holder: existing.holder_agent_id.clone(),
            });
        }
        self.track_locks.remove(track_id);
        Ok(EngineEvent::TrackLockChanged {
            project_id: self.project.id.clone(),
            track_id: track_id.to_string(),
            holder_agent_id: None,
            role: None,
        })
    }

    fn check_command_locks(
        &self,
        command: &TimelineCommand,
        meta: &CommandMeta,
    ) -> Result<(), CoreError> {
        match command {
            TimelineCommand::Batch { commands, .. } => {
                if commands.is_empty() {
                    return Err(CoreError::InvalidOperation(
                        "batch command must contain at least one sub-command".to_string(),
                    ));
                }
                if commands.len() > 256 {
                    return Err(CoreError::InvalidOperation(
                        "batch command exceeds maximum of 256 sub-commands".to_string(),
                    ));
                }
                for sub in commands {
                    if matches!(sub, TimelineCommand::Batch { .. }) {
                        return Err(CoreError::InvalidOperation(
                            "nested batch commands are not allowed".to_string(),
                        ));
                    }
                    if matches!(sub, TimelineCommand::UpdateMediaIntelligence { .. }) {
                        return Err(CoreError::InvalidOperation(
                            "update_media_intelligence cannot be inside a batch".to_string(),
                        ));
                    }
                    self.check_command_locks(sub, meta)?;
                }
                Ok(())
            }
            _ => {
                if let Some(track_id) = command.target_track_id() {
                    let track_is_locked = self
                        .project
                        .tracks
                        .iter()
                        .any(|t| t.id == track_id && t.locked);
                    if track_is_locked {
                        let allowed_user_unlock = meta.role == AgentRole::User
                            && matches!(command, TimelineCommand::SetTrackFlags { .. });
                        if !allowed_user_unlock {
                            return Err(CoreError::TrackLocked {
                                track_id: track_id.to_string(),
                                holder: "track_locked".to_string(),
                            });
                        }
                    }
                    if meta.role != AgentRole::User
                        && let Some(lock) = self.track_locks.get(track_id)
                        && lock.holder_agent_id != meta.agent_id
                    {
                        return Err(CoreError::TrackLocked {
                            track_id: track_id.to_string(),
                            holder: lock.holder_agent_id.clone(),
                        });
                    }
                }
                Ok(())
            }
        }
    }

    pub fn apply(
        &mut self,
        command: TimelineCommand,
        meta: CommandMeta,
    ) -> Result<(OpResult, EngineEvent), CoreError> {
        self.prune_expired_locks();
        self.check_command_locks(&command, &meta)?;

        let is_undoable = !matches!(command, TimelineCommand::UpdateMediaIntelligence { .. });
        let command_kind = command.kind_name().to_string();
        let before = if is_undoable {
            Some(self.project.clone())
        } else {
            None
        };
        let (summary, ui_anchor) = match self.apply_inner(command) {
            Ok(res) => res,
            Err(err) => {
                if let Some(snapshot) = before {
                    self.project = snapshot;
                }
                return Err(err);
            }
        };

        self.project.revision += 1;
        self.project.updated_at = Utc::now();

        self.project.activity_log.push(OpLogEntry {
            revision: self.project.revision,
            timestamp: self.project.updated_at,
            agent_id: meta.agent_id.clone(),
            role: meta.role,
            command_kind: command_kind.clone(),
            summary: summary.clone(),
            ui_anchor: ui_anchor.clone(),
        });
        if self.project.activity_log.len() > 200 {
            let overflow = self.project.activity_log.len() - 200;
            self.project.activity_log.drain(0..overflow);
        }

        if let Some(snapshot) = before {
            self.undo_stack.push(HistoryEntry {
                snapshot,
                summary: summary.clone(),
            });
            if self.undo_stack.len() > 100 {
                self.undo_stack.remove(0);
            }
            self.redo_stack.clear();
        }

        let result = OpResult {
            revision: self.project.revision,
            summary: summary.clone(),
            ui_anchor: ui_anchor.clone(),
        };

        let event = EngineEvent::OpCommitted {
            project_id: self.project.id.clone(),
            revision: self.project.revision,
            agent_id: meta.agent_id,
            role: meta.role,
            command_kind,
            summary,
            ui_anchor,
            project: Box::new(self.project.clone()),
        };

        Ok((result, event))
    }

    pub fn undo(&mut self) -> Result<EngineEvent, CoreError> {
        let entry = self.undo_stack.pop().ok_or(CoreError::NothingToUndo)?;
        let current = self.project.clone();
        let live_intel: HashMap<String, MediaIntelligenceManifest> = self
            .project
            .media_pool
            .iter()
            .map(|m| (m.id.clone(), m.intelligence.clone()))
            .collect();
        self.redo_stack.push(HistoryEntry {
            snapshot: current,
            summary: entry.summary.clone(),
        });
        let next_rev = self.project.revision + 1;
        let log = std::mem::take(&mut self.project.activity_log);
        self.project = entry.snapshot;
        for asset in &mut self.project.media_pool {
            if let Some(cur) = live_intel.get(&asset.id) {
                asset.intelligence = cur.clone();
            }
        }
        self.project.revision = next_rev;
        self.project.updated_at = Utc::now();
        self.project.activity_log = log;
        self.project.activity_log.push(OpLogEntry {
            revision: self.project.revision,
            timestamp: self.project.updated_at,
            agent_id: "user".to_string(),
            role: AgentRole::User,
            command_kind: "undo".to_string(),
            summary: format!("undo: {}", entry.summary),
            ui_anchor: "timeline".to_string(),
        });
        if self.project.activity_log.len() > 200 {
            let overflow = self.project.activity_log.len() - 200;
            self.project.activity_log.drain(0..overflow);
        }

        Ok(EngineEvent::ProjectReset {
            project_id: self.project.id.clone(),
            revision: self.project.revision,
            reason: format!("undo: {}", entry.summary),
            project: Box::new(self.project.clone()),
        })
    }

    pub fn redo(&mut self) -> Result<EngineEvent, CoreError> {
        let entry = self.redo_stack.pop().ok_or(CoreError::NothingToRedo)?;
        let current = self.project.clone();
        let live_intel: HashMap<String, MediaIntelligenceManifest> = self
            .project
            .media_pool
            .iter()
            .map(|m| (m.id.clone(), m.intelligence.clone()))
            .collect();
        self.undo_stack.push(HistoryEntry {
            snapshot: current,
            summary: entry.summary.clone(),
        });
        if self.undo_stack.len() > 100 {
            self.undo_stack.remove(0);
        }
        let next_rev = self.project.revision + 1;
        let log = std::mem::take(&mut self.project.activity_log);
        self.project = entry.snapshot;
        for asset in &mut self.project.media_pool {
            if let Some(cur) = live_intel.get(&asset.id) {
                asset.intelligence = cur.clone();
            }
        }
        self.project.revision = next_rev;
        self.project.updated_at = Utc::now();
        self.project.activity_log = log;
        self.project.activity_log.push(OpLogEntry {
            revision: self.project.revision,
            timestamp: self.project.updated_at,
            agent_id: "user".to_string(),
            role: AgentRole::User,
            command_kind: "redo".to_string(),
            summary: format!("redo: {}", entry.summary),
            ui_anchor: "timeline".to_string(),
        });
        if self.project.activity_log.len() > 200 {
            let overflow = self.project.activity_log.len() - 200;
            self.project.activity_log.drain(0..overflow);
        }

        Ok(EngineEvent::ProjectReset {
            project_id: self.project.id.clone(),
            revision: self.project.revision,
            reason: format!("redo: {}", entry.summary),
            project: Box::new(self.project.clone()),
        })
    }

    fn apply_inner(&mut self, command: TimelineCommand) -> Result<(String, String), CoreError> {
        match command {
            TimelineCommand::AddMedia { asset } => {
                if asset.id.trim().is_empty() {
                    return Err(CoreError::InvalidOperation(
                        "asset.id must not be empty".to_string(),
                    ));
                }
                validate_intelligence(&asset.intelligence)?;
                let label = asset.name.clone();
                if let Some(existing) = self
                    .project
                    .media_pool
                    .iter_mut()
                    .find(|m| m.id == asset.id)
                {
                    *existing = asset;
                } else {
                    self.project.media_pool.push(asset);
                }
                Ok((format!("added media '{label}'"), "media.pool".to_string()))
            }
            TimelineCommand::RemoveMedia { media_id } => {
                if media_id.trim().is_empty() {
                    return Err(CoreError::InvalidOperation(
                        "media_id must not be empty".to_string(),
                    ));
                }
                let idx = self
                    .project
                    .media_pool
                    .iter()
                    .position(|m| m.id == media_id)
                    .ok_or_else(|| CoreError::MediaNotFound(media_id.clone()))?;
                let removed = self.project.media_pool.remove(idx);
                let mut removed_clips = 0usize;
                for track in &mut self.project.tracks {
                    let before = track.clips.len();
                    track.clips.retain(|c| {
                        !matches!(
                            &c.payload,
                            ClipKind::Media { media_id: mid, .. } if mid == &media_id
                        )
                    });
                    removed_clips += before - track.clips.len();
                }
                let summary = if removed_clips > 0 {
                    format!(
                        "removed media '{}' and {} clip(s)",
                        removed.name, removed_clips
                    )
                } else {
                    format!("removed media '{}'", removed.name)
                };
                Ok((summary, "media.pool".to_string()))
            }
            TimelineCommand::UpdateMediaIntelligence {
                media_id,
                intelligence,
            } => {
                validate_intelligence(&intelligence)?;
                let asset = self
                    .project
                    .media_pool
                    .iter_mut()
                    .find(|m| m.id == media_id)
                    .ok_or_else(|| CoreError::MediaNotFound(media_id.clone()))?;
                asset.intelligence = intelligence;
                Ok((
                    format!("analyzed '{}'", asset.name),
                    format!("media.pool.{media_id}"),
                ))
            }
            TimelineCommand::PlaceClip { track_id, clip } => {
                ensure_finite_span(clip.start_sec, clip.duration_sec, clip.source_in_sec)?;
                validate_transform(&clip.transform)?;
                validate_audio_settings(&clip.audio)?;
                match &clip.payload {
                    ClipKind::Caption { caption } => validate_caption_block(caption)?,
                    ClipKind::Grade { grade } => validate_color_grade(grade)?,
                    ClipKind::Gfx { gfx } => validate_gfx_instance(gfx)?,
                    ClipKind::Media { .. } => {}
                }
                if clip.id.trim().is_empty() {
                    return Err(CoreError::InvalidOperation(
                        "clip id must not be empty".to_string(),
                    ));
                }
                if !(clip.duration_sec > 0.0 && clip.start_sec >= 0.0 && clip.source_in_sec >= 0.0)
                {
                    return Err(CoreError::InvalidOperation(
                        "clip start_sec >= 0, source_in_sec >= 0, and duration_sec > 0 required"
                            .to_string(),
                    ));
                }
                let track = self
                    .project
                    .tracks
                    .iter_mut()
                    .find(|t| t.id == track_id)
                    .ok_or_else(|| CoreError::TrackNotFound(track_id.clone()))?;
                if track.clips.iter().any(|c| c.id == clip.id) {
                    return Err(CoreError::InvalidOperation(format!(
                        "duplicate clip id '{}'",
                        clip.id
                    )));
                }
                let label = clip.label.clone();
                track.clips.push(clip);
                track
                    .clips
                    .sort_by(|a, b| a.start_sec.total_cmp(&b.start_sec));
                Ok((
                    format!("placed '{label}' on {}", track.code),
                    format!("timeline.track.{track_id}"),
                ))
            }
            TimelineCommand::SplitClip {
                track_id,
                clip_id,
                split_at_timeline_sec,
                right_clip_id,
            } => {
                ensure_finite(split_at_timeline_sec, "split_at_timeline_sec")?;
                if right_clip_id.trim().is_empty() || right_clip_id == clip_id {
                    return Err(CoreError::InvalidOperation(
                        "right_clip_id must be non-empty and distinct from clip_id".to_string(),
                    ));
                }
                let track = self
                    .project
                    .tracks
                    .iter_mut()
                    .find(|t| t.id == track_id)
                    .ok_or_else(|| CoreError::TrackNotFound(track_id.clone()))?;
                if track.clips.iter().any(|c| c.id == right_clip_id) {
                    return Err(CoreError::InvalidOperation(format!(
                        "duplicate right_clip_id '{right_clip_id}'"
                    )));
                }
                let idx = track
                    .clips
                    .iter()
                    .position(|c| c.id == clip_id)
                    .ok_or_else(|| CoreError::ClipNotFound(clip_id.clone()))?;
                let left = &mut track.clips[idx];
                ensure_finite_span(left.start_sec, left.duration_sec, left.source_in_sec)?;
                let clip_end = left.start_sec + left.duration_sec;
                if !(split_at_timeline_sec > left.start_sec && split_at_timeline_sec < clip_end) {
                    return Err(CoreError::InvalidOperation(
                        "split point must lie strictly inside clip bounds".to_string(),
                    ));
                }
                let left_duration = split_at_timeline_sec - left.start_sec;
                let right_duration = clip_end - split_at_timeline_sec;
                let right_source_in = left.source_in_sec + left_duration;
                ensure_finite_span(split_at_timeline_sec, right_duration, right_source_in)?;
                let mut right = left.clone();
                right.id = right_clip_id;
                right.start_sec = split_at_timeline_sec;
                right.duration_sec = right_duration;
                right.source_in_sec = right_source_in;
                left.duration_sec = left_duration;

                if let (
                    ClipKind::Caption { caption: left_cap },
                    ClipKind::Caption { caption: right_cap },
                ) = (&mut left.payload, &mut right.payload)
                {
                    left_cap.words = left_cap
                        .words
                        .iter()
                        .filter(|w| w.start_sec < split_at_timeline_sec)
                        .cloned()
                        .map(|mut w| {
                            w.end_sec = w.end_sec.min(split_at_timeline_sec);
                            w
                        })
                        .collect();
                    right_cap.words = right_cap
                        .words
                        .iter()
                        .filter(|w| w.end_sec > split_at_timeline_sec)
                        .cloned()
                        .map(|mut w| {
                            w.start_sec = w.start_sec.max(split_at_timeline_sec);
                            w
                        })
                        .collect();
                }

                let label = left.label.clone();
                track.clips.insert(idx + 1, right);
                track
                    .clips
                    .sort_by(|a, b| a.start_sec.total_cmp(&b.start_sec));
                Ok((
                    format!("split '{label}' at {split_at_timeline_sec:.2}s"),
                    format!("timeline.clip.{clip_id}"),
                ))
            }
            TimelineCommand::TrimClip {
                track_id,
                clip_id,
                new_start_sec,
                new_duration_sec,
                new_source_in_sec,
            } => {
                if let Some(s) = new_start_sec {
                    ensure_finite(s, "new_start_sec")?;
                    if s < 0.0 {
                        return Err(CoreError::InvalidOperation("start_sec < 0".to_string()));
                    }
                }
                if let Some(d) = new_duration_sec {
                    ensure_finite(d, "new_duration_sec")?;
                    if d <= 0.0 {
                        return Err(CoreError::InvalidOperation(
                            "duration_sec must be > 0".to_string(),
                        ));
                    }
                }
                if let Some(si) = new_source_in_sec {
                    ensure_finite(si, "new_source_in_sec")?;
                    if si < 0.0 {
                        return Err(CoreError::InvalidOperation("source_in_sec < 0".to_string()));
                    }
                }
                let track = self
                    .project
                    .tracks
                    .iter_mut()
                    .find(|t| t.id == track_id)
                    .ok_or_else(|| CoreError::TrackNotFound(track_id.clone()))?;
                let clip = track
                    .clips
                    .iter_mut()
                    .find(|c| c.id == clip_id)
                    .ok_or_else(|| CoreError::ClipNotFound(clip_id.clone()))?;
                let old_start = clip.start_sec;
                let eff_start = new_start_sec.unwrap_or(old_start);
                let eff_dur = new_duration_sec.unwrap_or(clip.duration_sec);
                let eff_src_in = new_source_in_sec.unwrap_or(clip.source_in_sec);
                ensure_finite_span(eff_start, eff_dur, eff_src_in)?;
                clip.start_sec = eff_start;
                clip.duration_sec = eff_dur;
                clip.source_in_sec = eff_src_in;
                let delta = eff_start - old_start;
                if delta.abs() > 1e-9
                    && let ClipKind::Caption { caption } = &mut clip.payload
                {
                    for w in &mut caption.words {
                        w.start_sec = (w.start_sec + delta).max(0.0);
                        w.end_sec = (w.end_sec + delta).max(w.start_sec);
                    }
                }
                let label = clip.label.clone();
                track
                    .clips
                    .sort_by(|a, b| a.start_sec.total_cmp(&b.start_sec));
                Ok((
                    format!("trimmed '{label}'"),
                    format!("timeline.clip.{clip_id}"),
                ))
            }
            TimelineCommand::DeleteClip { track_id, clip_id } => {
                let track = self
                    .project
                    .tracks
                    .iter_mut()
                    .find(|t| t.id == track_id)
                    .ok_or_else(|| CoreError::TrackNotFound(track_id.clone()))?;
                let idx = track
                    .clips
                    .iter()
                    .position(|c| c.id == clip_id)
                    .ok_or_else(|| CoreError::ClipNotFound(clip_id.clone()))?;
                let removed = track.clips.remove(idx);
                Ok((
                    format!("deleted '{}' from {}", removed.label, track.code),
                    format!("timeline.track.{track_id}"),
                ))
            }
            TimelineCommand::UpdateClipTransform {
                track_id,
                clip_id,
                transform,
            } => {
                validate_transform(&transform)?;
                let track = self
                    .project
                    .tracks
                    .iter_mut()
                    .find(|t| t.id == track_id)
                    .ok_or_else(|| CoreError::TrackNotFound(track_id.clone()))?;
                let clip = track
                    .clips
                    .iter_mut()
                    .find(|c| c.id == clip_id)
                    .ok_or_else(|| CoreError::ClipNotFound(clip_id.clone()))?;
                clip.transform = transform;
                Ok((
                    format!("updated transform on '{}'", clip.label),
                    "program.stage".to_string(),
                ))
            }
            TimelineCommand::SetColorGrade {
                grade,
                track_id,
                clip_id,
            } => {
                validate_color_grade(&grade)?;
                match (track_id, clip_id) {
                    (Some(tid), Some(cid)) => {
                        let track = self
                            .project
                            .tracks
                            .iter_mut()
                            .find(|t| t.id == tid)
                            .ok_or(CoreError::TrackNotFound(tid))?;
                        let clip = track
                            .clips
                            .iter_mut()
                            .find(|c| c.id == cid)
                            .ok_or_else(|| CoreError::ClipNotFound(cid.clone()))?;
                        if !matches!(clip.payload, ClipKind::Grade { .. }) {
                            return Err(CoreError::InvalidOperation(format!(
                                "clip '{cid}' is not a Grade clip"
                            )));
                        }
                        clip.payload = ClipKind::Grade { grade };
                    }
                    (None, None) => {
                        self.project.active_grade = grade;
                    }
                    _ => {
                        return Err(CoreError::InvalidOperation(
                            "SetColorGrade requires both track_id and clip_id, or neither"
                                .to_string(),
                        ));
                    }
                }
                Ok((
                    "updated color grade & LUT".to_string(),
                    "color.inspector".to_string(),
                ))
            }
            TimelineCommand::UpdateCaptionClip {
                track_id,
                clip_id,
                caption,
            } => {
                validate_caption_block(&caption)?;
                let track = self
                    .project
                    .tracks
                    .iter_mut()
                    .find(|t| t.id == track_id)
                    .ok_or_else(|| CoreError::TrackNotFound(track_id.clone()))?;
                let clip = track
                    .clips
                    .iter_mut()
                    .find(|c| c.id == clip_id)
                    .ok_or_else(|| CoreError::ClipNotFound(clip_id.clone()))?;
                if !matches!(clip.payload, ClipKind::Caption { .. }) {
                    return Err(CoreError::InvalidOperation(format!(
                        "clip '{clip_id}' is not a Caption clip"
                    )));
                }
                if !caption.words.is_empty() {
                    let preview: String = caption
                        .words
                        .iter()
                        .take(6)
                        .map(|w| w.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    let trimmed: String = preview.chars().take(48).collect();
                    if !trimmed.trim().is_empty() {
                        clip.label = trimmed;
                    }
                }
                clip.payload = ClipKind::Caption { caption };
                Ok((
                    format!("updated captions on '{}'", clip.label),
                    "captions.inspector".to_string(),
                ))
            }
            TimelineCommand::PlaceGfx {
                track_id,
                clip_id,
                label,
                start_sec,
                duration_sec,
                gfx,
            } => {
                ensure_finite_span(start_sec, duration_sec, 0.0)?;
                validate_gfx_instance(&gfx)?;
                if clip_id.trim().is_empty() || !(duration_sec > 0.0 && start_sec >= 0.0) {
                    return Err(CoreError::InvalidOperation(
                        "GFX clip_id non-empty, start_sec >= 0, and duration_sec > 0 required"
                            .to_string(),
                    ));
                }
                let track = self
                    .project
                    .tracks
                    .iter_mut()
                    .find(|t| t.id == track_id)
                    .ok_or_else(|| CoreError::TrackNotFound(track_id.clone()))?;
                if track.clips.iter().any(|c| c.id == clip_id) {
                    return Err(CoreError::InvalidOperation(format!(
                        "duplicate clip id '{clip_id}'"
                    )));
                }
                track.clips.push(Clip {
                    id: clip_id,
                    label: label.clone(),
                    start_sec,
                    duration_sec,
                    source_in_sec: 0.0,
                    transform: Transform2D::default(),
                    audio: AudioSettings::default(),
                    payload: ClipKind::Gfx { gfx },
                });
                track
                    .clips
                    .sort_by(|a, b| a.start_sec.total_cmp(&b.start_sec));
                Ok((
                    format!("placed GFX '{label}' on {}", track.code),
                    format!("timeline.track.{track_id}"),
                ))
            }
            TimelineCommand::UpdateGfxProps {
                track_id,
                clip_id,
                props,
            } => {
                let track = self
                    .project
                    .tracks
                    .iter_mut()
                    .find(|t| t.id == track_id)
                    .ok_or_else(|| CoreError::TrackNotFound(track_id.clone()))?;
                let clip = track
                    .clips
                    .iter_mut()
                    .find(|c| c.id == clip_id)
                    .ok_or_else(|| CoreError::ClipNotFound(clip_id.clone()))?;
                match &mut clip.payload {
                    ClipKind::Gfx { gfx } => {
                        gfx.props = props;
                    }
                    _ => {
                        return Err(CoreError::InvalidOperation(format!(
                            "clip '{clip_id}' is not a GFX clip"
                        )));
                    }
                }
                Ok((
                    format!("updated GFX props on '{}'", clip.label),
                    "program.stage".to_string(),
                ))
            }
            TimelineCommand::SetAudioSettings {
                track_id,
                clip_id,
                audio,
            } => {
                validate_audio_settings(&audio)?;
                let track = self
                    .project
                    .tracks
                    .iter_mut()
                    .find(|t| t.id == track_id)
                    .ok_or_else(|| CoreError::TrackNotFound(track_id.clone()))?;
                let clip = track
                    .clips
                    .iter_mut()
                    .find(|c| c.id == clip_id)
                    .ok_or_else(|| CoreError::ClipNotFound(clip_id.clone()))?;
                clip.audio = audio;
                Ok((
                    format!("updated audio mix on '{}'", clip.label),
                    "audio.mixer".to_string(),
                ))
            }
            TimelineCommand::SetTrackFlags {
                track_id,
                muted,
                solo,
                locked,
            } => {
                let track = self
                    .project
                    .tracks
                    .iter_mut()
                    .find(|t| t.id == track_id)
                    .ok_or_else(|| CoreError::TrackNotFound(track_id.clone()))?;
                if let Some(m) = muted {
                    track.muted = m;
                }
                if let Some(s) = solo {
                    track.solo = s;
                }
                if let Some(l) = locked {
                    track.locked = l;
                }
                Ok((
                    format!("updated flags on {}", track.code),
                    format!("timeline.track.{track_id}"),
                ))
            }
            TimelineCommand::SetBrandPlaybook { brand_playbook_id } => {
                if brand_playbook_id.trim().is_empty() {
                    return Err(CoreError::InvalidOperation(
                        "brand_playbook_id must not be empty".to_string(),
                    ));
                }
                self.project.brand_playbook_id = brand_playbook_id.clone();
                Ok((
                    format!("switched brand playbook to '{brand_playbook_id}'"),
                    "brand.playbook".to_string(),
                ))
            }
            TimelineCommand::SetCanvas { canvas } => {
                if !(16..=7680).contains(&canvas.width) || !(16..=7680).contains(&canvas.height) {
                    return Err(CoreError::InvalidOperation(
                        "canvas width/height must be between 16 and 7680".to_string(),
                    ));
                }
                if canvas.fps_num == 0 || canvas.fps_den == 0 {
                    return Err(CoreError::InvalidOperation(
                        "canvas fps_num and fps_den must be > 0".to_string(),
                    ));
                }
                let summary = format!(
                    "updated canvas to {}x{} ({:?})",
                    canvas.width, canvas.height, canvas.aspect_preset
                );
                self.project.canvas = canvas;
                Ok((summary, "program.stage".to_string()))
            }
            TimelineCommand::UpdateMediaRole {
                media_id,
                role_hint,
            } => {
                let cleaned = role_hint.trim().to_string();
                if cleaned.is_empty() {
                    return Err(CoreError::InvalidOperation(
                        "role_hint must not be empty".to_string(),
                    ));
                }
                let asset = self
                    .project
                    .media_pool
                    .iter_mut()
                    .find(|m| m.id == media_id)
                    .ok_or_else(|| CoreError::MediaNotFound(media_id.clone()))?;
                asset.role_hint = cleaned.clone();
                Ok((
                    format!("updated media role for '{}' to '{cleaned}'", asset.name),
                    format!("media.pool.{media_id}"),
                ))
            }
            TimelineCommand::Batch { commands, summary } => {
                let count = commands.len();
                let mut first_summary = String::new();
                let mut distinct_anchors: Vec<String> = Vec::new();
                for sub in commands {
                    let (sub_summary, sub_anchor) = self.apply_inner(sub)?;
                    if first_summary.is_empty() {
                        first_summary = sub_summary;
                    }
                    if !distinct_anchors.contains(&sub_anchor) {
                        distinct_anchors.push(sub_anchor);
                    }
                }
                let final_summary = summary
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| {
                        if count == 1 {
                            first_summary
                        } else {
                            format!("batch ({count} ops): {first_summary}")
                        }
                    });
                let final_anchor = if distinct_anchors.len() == 1 {
                    distinct_anchors.remove(0)
                } else {
                    "timeline".to_string()
                };
                Ok((final_summary, final_anchor))
            }
        }
    }
}

pub fn save_project_atomic(project: &Project, path: &Path) -> Result<(), CoreError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|e| CoreError::Persistence(e.to_string()))?;
    }
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project.json");
    let tmp_path = path.with_file_name(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));
    let json =
        serde_json::to_string_pretty(project).map_err(|e| CoreError::Persistence(e.to_string()))?;
    {
        use std::io::Write;
        let mut file =
            fs::File::create(&tmp_path).map_err(|e| CoreError::Persistence(e.to_string()))?;
        if let Err(err) = file
            .write_all(json.as_bytes())
            .and_then(|_| file.sync_all())
        {
            let _ = fs::remove_file(&tmp_path);
            return Err(CoreError::Persistence(err.to_string()));
        }
    }
    if let Err(err) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(CoreError::Persistence(err.to_string()));
    }
    Ok(())
}

pub fn load_project(path: &Path) -> Result<Project, CoreError> {
    let raw = fs::read_to_string(path).map_err(|e| CoreError::Persistence(e.to_string()))?;
    serde_json::from_str(&raw).map_err(|e| CoreError::Persistence(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AspectPreset, ColorGrade, RgbTriplet};

    fn sample_clip(id: &str, start: f64, duration: f64) -> Clip {
        Clip {
            id: id.to_string(),
            label: format!("Clip {id}"),
            start_sec: start,
            duration_sec: duration,
            source_in_sec: 0.0,
            transform: Transform2D::default(),
            audio: AudioSettings::default(),
            payload: ClipKind::Media {
                media_id: "m1".to_string(),
                ken_burns: false,
            },
        }
    }

    #[test]
    fn default_project_has_seven_semantic_tracks() {
        let project = Project::new_default("ai_tools_reel_v1", AspectPreset::Vertical9x16);
        assert_eq!(project.tracks.len(), 7);
        let codes: Vec<&str> = project.tracks.iter().map(|t| t.code.as_str()).collect();
        assert_eq!(codes, vec!["T1", "G1", "V2", "V1", "FX", "A1", "A2"]);
        assert_eq!(project.canvas.width, 1080);
        assert_eq!(project.canvas.height, 1920);
    }

    #[test]
    fn place_split_trim_and_undo_redo_cycle() {
        let mut engine =
            ProjectEngine::new(Project::new_default("test", AspectPreset::Vertical9x16));
        let meta = CommandMeta::default();

        engine
            .apply(
                TimelineCommand::PlaceClip {
                    track_id: "v1".to_string(),
                    clip: sample_clip("c1", 0.0, 10.0),
                },
                meta.clone(),
            )
            .expect("place clip");

        engine
            .apply(
                TimelineCommand::SplitClip {
                    track_id: "v1".to_string(),
                    clip_id: "c1".to_string(),
                    split_at_timeline_sec: 4.0,
                    right_clip_id: "c2".to_string(),
                },
                meta.clone(),
            )
            .expect("split clip");

        let v1 = engine
            .project()
            .tracks
            .iter()
            .find(|t| t.id == "v1")
            .unwrap();
        assert_eq!(v1.clips.len(), 2);
        assert!((v1.clips[0].duration_sec - 4.0).abs() < 1e-6);
        assert!((v1.clips[1].start_sec - 4.0).abs() < 1e-6);
        assert!((v1.clips[1].duration_sec - 6.0).abs() < 1e-6);
        assert!((v1.clips[1].source_in_sec - 4.0).abs() < 1e-6);

        engine.undo().expect("undo split");
        let v1_after_undo = engine
            .project()
            .tracks
            .iter()
            .find(|t| t.id == "v1")
            .unwrap();
        assert_eq!(v1_after_undo.clips.len(), 1);
        assert!((v1_after_undo.clips[0].duration_sec - 10.0).abs() < 1e-6);

        engine.redo().expect("redo split");
        let v1_after_redo = engine
            .project()
            .tracks
            .iter()
            .find(|t| t.id == "v1")
            .unwrap();
        assert_eq!(v1_after_redo.clips.len(), 2);
    }

    #[test]
    fn track_locks_block_competing_agents() {
        let mut engine = ProjectEngine::new(Project::new_default(
            "lock_test",
            AspectPreset::Vertical9x16,
        ));
        engine
            .acquire_track_lock("t1", "caps-01", AgentRole::Caps)
            .expect("acquire lock");

        let err = engine
            .apply(
                TimelineCommand::PlaceClip {
                    track_id: "t1".to_string(),
                    clip: sample_clip("bad", 0.0, 2.0),
                },
                CommandMeta {
                    agent_id: "razor-01".to_string(),
                    role: AgentRole::Razor,
                    reason: None,
                },
            )
            .unwrap_err();

        assert!(matches!(err, CoreError::TrackLocked { .. }));
    }

    #[test]
    fn color_grade_and_json_round_trip() {
        let mut engine = ProjectEngine::new(Project::new_default(
            "grade_test",
            AspectPreset::Landscape16x9,
        ));
        let grade = ColorGrade {
            gain: RgbTriplet {
                r: 0.12,
                g: 0.04,
                b: -0.05,
            },
            contrast: 1.15,
            saturation: 1.18,
            ..ColorGrade::default()
        };
        engine
            .apply(
                TimelineCommand::SetColorGrade {
                    grade: grade.clone(),
                    track_id: None,
                    clip_id: None,
                },
                CommandMeta {
                    agent_id: "grade-01".to_string(),
                    role: AgentRole::Grade,
                    reason: Some("boost skin warmth".to_string()),
                },
            )
            .expect("set grade");

        let json = serde_json::to_string(engine.project()).unwrap();
        let decoded: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.active_grade, grade);
        assert_eq!(decoded.activity_log.len(), 1);
        assert_eq!(decoded.activity_log[0].agent_id, "grade-01");
    }

    #[test]
    fn update_gfx_props_and_brand_backwards_compat() {
        let mut engine =
            ProjectEngine::new(Project::new_default("gfx_test", AspectPreset::Vertical9x16));
        engine
            .apply(
                TimelineCommand::PlaceGfx {
                    track_id: "g1".to_string(),
                    clip_id: "g_1".to_string(),
                    label: "Stat Card".to_string(),
                    start_sec: 0.5,
                    duration_sec: 3.0,
                    gfx: crate::model::GfxInstance {
                        asset_id: Some("stat-card-solar".to_string()),
                        props: serde_json::json!({ "value": "10 HRS" }),
                        inline_html: None,
                    },
                },
                CommandMeta {
                    agent_id: "gfx-01".to_string(),
                    role: AgentRole::Gfx,
                    reason: None,
                },
            )
            .expect("place gfx");

        engine
            .apply(
                TimelineCommand::UpdateGfxProps {
                    track_id: "g1".to_string(),
                    clip_id: "g_1".to_string(),
                    props: serde_json::json!({ "value": "15 HRS" }),
                },
                CommandMeta::default(),
            )
            .expect("update gfx props");

        let g1 = engine
            .project()
            .tracks
            .iter()
            .find(|t| t.id == "g1")
            .unwrap();
        match &g1.clips[0].payload {
            ClipKind::Gfx { gfx } => assert_eq!(gfx.props["value"], "15 HRS"),
            _ => panic!("expected gfx clip"),
        }
        assert_eq!(engine.project().activity_log.len(), 2);

        engine.undo().expect("undo gfx update");
        assert_eq!(engine.project().activity_log.len(), 3);
        assert_eq!(engine.project().activity_log[2].command_kind, "undo");
    }

    #[test]
    fn enforces_user_track_lock_and_isolates_media_intelligence_from_undo() {
        let mut engine = ProjectEngine::new(Project::new_default(
            "lock_and_undo",
            AspectPreset::Vertical9x16,
        ));
        engine
            .apply(
                TimelineCommand::AddMedia {
                    asset: crate::model::MediaAsset {
                        id: "m1".into(),
                        name: "clip.mp4".into(),
                        kind: crate::model::MediaKind::Video,
                        role_hint: "a_roll".into(),
                        path: "/tmp/clip.mp4".into(),
                        added_at: Utc::now(),
                        intelligence: crate::model::MediaIntelligenceManifest::default(),
                    },
                },
                CommandMeta::default(),
            )
            .expect("add media");

        engine
            .apply(
                TimelineCommand::PlaceClip {
                    track_id: "v1".into(),
                    clip: sample_clip("c1", 0.0, 2.0),
                },
                CommandMeta::default(),
            )
            .expect("place clip");
        engine.undo().expect("undo place");
        assert!(engine.can_redo());

        // Background intelligence update must NOT wipe redo_stack or enter undo_stack
        engine
            .apply(
                TimelineCommand::UpdateMediaIntelligence {
                    media_id: "m1".into(),
                    intelligence: crate::model::MediaIntelligenceManifest {
                        analyzed: true,
                        duration_sec: 2.0,
                        ..crate::model::MediaIntelligenceManifest::default()
                    },
                },
                CommandMeta::default(),
            )
            .expect("update intel");
        assert!(engine.can_redo());

        // UI track lock blocks mutations until unlocked
        engine
            .apply(
                TimelineCommand::SetTrackFlags {
                    track_id: "v1".into(),
                    muted: None,
                    solo: None,
                    locked: Some(true),
                },
                CommandMeta::default(),
            )
            .expect("lock v1");
        let err = engine
            .apply(
                TimelineCommand::PlaceClip {
                    track_id: "v1".into(),
                    clip: sample_clip("c2", 0.0, 1.0),
                },
                CommandMeta::default(),
            )
            .unwrap_err();
        assert!(matches!(err, CoreError::TrackLocked { .. }));

        // Non-User agents cannot unlock a user-locked track
        let agent_unlock = engine
            .apply(
                TimelineCommand::SetTrackFlags {
                    track_id: "v1".into(),
                    muted: None,
                    solo: None,
                    locked: Some(false),
                },
                CommandMeta {
                    agent_id: "razor-01".into(),
                    role: AgentRole::Razor,
                    reason: None,
                },
            )
            .unwrap_err();
        assert!(matches!(agent_unlock, CoreError::TrackLocked { .. }));

        // Undo & Redo preserve media_pool intelligence (transcript, peaks, proxy_path)
        engine
            .apply(
                TimelineCommand::SetTrackFlags {
                    track_id: "v1".into(),
                    muted: None,
                    solo: None,
                    locked: Some(false),
                },
                CommandMeta::default(),
            )
            .expect("user unlock");
        engine
            .apply(
                TimelineCommand::PlaceClip {
                    track_id: "v1".into(),
                    clip: sample_clip("c_undo_intel", 0.0, 1.5),
                },
                CommandMeta::default(),
            )
            .expect("place clip");
        engine
            .apply(
                TimelineCommand::UpdateMediaIntelligence {
                    media_id: "m1".into(),
                    intelligence: crate::model::MediaIntelligenceManifest {
                        analyzed: true,
                        duration_sec: 2.0,
                        proxy_path: Some("/tmp/proxy.mp4".into()),
                        transcript: vec![crate::model::TranscriptWord {
                            word: "Hello".into(),
                            start_sec: 0.1,
                            end_sec: 0.4,
                        }],
                        ..crate::model::MediaIntelligenceManifest::default()
                    },
                },
                CommandMeta::default(),
            )
            .expect("enrich intelligence");
        engine.undo().expect("undo clip");
        let m1 = engine
            .project()
            .media_pool
            .iter()
            .find(|m| m.id == "m1")
            .unwrap();
        assert_eq!(
            m1.intelligence.proxy_path.as_deref(),
            Some("/tmp/proxy.mp4")
        );
        assert_eq!(m1.intelligence.transcript.len(), 1);
    }

    #[test]
    fn rejects_nan_duplicate_ids_and_partitions_caption_split_and_clip_grade() {
        let mut engine = ProjectEngine::new(Project::new_default(
            "validation_test",
            AspectPreset::Vertical9x16,
        ));

        let mut bad_clip = sample_clip("c_nan", 0.0, 2.0);
        bad_clip.start_sec = f64::NAN;
        assert!(
            engine
                .apply(
                    TimelineCommand::PlaceClip {
                        track_id: "v1".into(),
                        clip: bad_clip,
                    },
                    CommandMeta::default(),
                )
                .is_err()
        );

        let cap_clip = Clip {
            id: "cap1".into(),
            label: "Captions".into(),
            start_sec: 0.0,
            duration_sec: 4.0,
            source_in_sec: 0.0,
            transform: Transform2D::default(),
            audio: AudioSettings::default(),
            payload: ClipKind::Caption {
                caption: crate::model::CaptionBlock {
                    words: vec![
                        crate::model::CaptionWord {
                            text: "One".into(),
                            start_sec: 0.2,
                            end_sec: 1.0,
                        },
                        crate::model::CaptionWord {
                            text: "Two".into(),
                            start_sec: 2.5,
                            end_sec: 3.5,
                        },
                    ],
                    preset: crate::model::CaptionPreset::NeonGlow,
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
                CommandMeta::default(),
            )
            .expect("place cap");
        engine
            .apply(
                TimelineCommand::SplitClip {
                    track_id: "t1".into(),
                    clip_id: "cap1".into(),
                    split_at_timeline_sec: 2.0,
                    right_clip_id: "cap2".into(),
                },
                CommandMeta::default(),
            )
            .expect("split cap");

        let t1 = engine
            .project()
            .tracks
            .iter()
            .find(|t| t.id == "t1")
            .unwrap();
        match (&t1.clips[0].payload, &t1.clips[1].payload) {
            (ClipKind::Caption { caption: l }, ClipKind::Caption { caption: r }) => {
                assert_eq!(l.words.len(), 1);
                assert_eq!(l.words[0].text, "One");
                assert_eq!(r.words.len(), 1);
                assert_eq!(r.words[0].text, "Two");
            }
            _ => panic!("expected caption clips"),
        }

        // Clip-scoped grade must not overwrite global active_grade
        engine
            .apply(
                TimelineCommand::PlaceClip {
                    track_id: "fx".into(),
                    clip: Clip {
                        id: "fx1".into(),
                        label: "Warm".into(),
                        start_sec: 0.0,
                        duration_sec: 2.0,
                        source_in_sec: 0.0,
                        transform: Transform2D::default(),
                        audio: AudioSettings::default(),
                        payload: ClipKind::Grade {
                            grade: ColorGrade::default(),
                        },
                    },
                },
                CommandMeta::default(),
            )
            .expect("place fx");
        let custom = ColorGrade {
            contrast: 1.4,
            ..ColorGrade::default()
        };
        engine
            .apply(
                TimelineCommand::SetColorGrade {
                    grade: custom.clone(),
                    track_id: Some("fx".into()),
                    clip_id: Some("fx1".into()),
                },
                CommandMeta::default(),
            )
            .expect("clip grade");
        assert!((engine.project().active_grade.contrast - 1.0).abs() < 1e-6);

        // SetColorGrade rejects partial (Some, None) tuples
        assert!(
            engine
                .apply(
                    TimelineCommand::SetColorGrade {
                        grade: custom.clone(),
                        track_id: Some("fx".into()),
                        clip_id: None,
                    },
                    CommandMeta::default(),
                )
                .is_err()
        );

        // SetColorGrade rejects non-Grade target clips (e.g., Media clip on v1)
        engine
            .apply(
                TimelineCommand::PlaceClip {
                    track_id: "v1".into(),
                    clip: sample_clip("v_media", 0.0, 2.0),
                },
                CommandMeta::default(),
            )
            .expect("place media clip");
        assert!(
            engine
                .apply(
                    TimelineCommand::SetColorGrade {
                        grade: custom.clone(),
                        track_id: Some("v1".into()),
                        clip_id: Some("v_media".into()),
                    },
                    CommandMeta::default(),
                )
                .is_err()
        );

        // Project-level SetColorGrade respects FX track lock
        engine
            .apply(
                TimelineCommand::SetTrackFlags {
                    track_id: "fx".into(),
                    muted: None,
                    solo: None,
                    locked: Some(true),
                },
                CommandMeta::default(),
            )
            .expect("lock fx");
        let fx_err = engine
            .apply(
                TimelineCommand::SetColorGrade {
                    grade: custom,
                    track_id: None,
                    clip_id: None,
                },
                CommandMeta {
                    agent_id: "grade-01".into(),
                    role: AgentRole::Grade,
                    reason: None,
                },
            )
            .unwrap_err();
        assert!(matches!(fx_err, CoreError::TrackLocked { .. }));
    }

    #[test]
    fn enforces_gfx_html_limit_non_empty_ids_float_overflow_and_remove_media() {
        let mut engine = ProjectEngine::new(Project::new_default(
            "hardening_test",
            AspectPreset::Vertical9x16,
        ));
        let meta = CommandMeta::default();

        // 1. Oversized inline_html rejected on PlaceGfx and PlaceClip
        let oversized = "x".repeat(MAX_GFX_HTML_BYTES + 1);
        assert!(
            engine
                .apply(
                    TimelineCommand::PlaceGfx {
                        track_id: "g1".into(),
                        clip_id: "g_big".into(),
                        label: "Too Big".into(),
                        start_sec: 0.0,
                        duration_sec: 2.0,
                        gfx: GfxInstance {
                            asset_id: None,
                            props: serde_json::json!({}),
                            inline_html: Some(oversized.clone()),
                        },
                    },
                    meta.clone(),
                )
                .is_err()
        );
        let big_gfx_clip = Clip {
            id: "g_big_clip".into(),
            label: "Too Big Clip".into(),
            start_sec: 0.0,
            duration_sec: 2.0,
            source_in_sec: 0.0,
            transform: Transform2D::default(),
            audio: AudioSettings::default(),
            payload: ClipKind::Gfx {
                gfx: GfxInstance {
                    asset_id: None,
                    props: serde_json::json!({}),
                    inline_html: Some(oversized),
                },
            },
        };
        assert!(
            engine
                .apply(
                    TimelineCommand::PlaceClip {
                        track_id: "g1".into(),
                        clip: big_gfx_clip,
                    },
                    meta.clone(),
                )
                .is_err()
        );

        // 2. Empty / whitespace-only asset.id and brand_playbook_id rejected
        assert!(
            engine
                .apply(
                    TimelineCommand::AddMedia {
                        asset: crate::model::MediaAsset {
                            id: "   ".into(),
                            name: "bad.mp4".into(),
                            kind: crate::model::MediaKind::Video,
                            role_hint: "a_roll".into(),
                            path: "/tmp/bad.mp4".into(),
                            added_at: Utc::now(),
                            intelligence: MediaIntelligenceManifest::default(),
                        },
                    },
                    meta.clone(),
                )
                .is_err()
        );
        assert!(
            engine
                .apply(
                    TimelineCommand::SetBrandPlaybook {
                        brand_playbook_id: " \t ".into(),
                    },
                    meta.clone(),
                )
                .is_err()
        );

        // 3. Float sum overflow (f64::MAX + f64::MAX = inf) rejected
        assert!(
            engine
                .apply(
                    TimelineCommand::PlaceClip {
                        track_id: "v1".into(),
                        clip: sample_clip("c_ovf", f64::MAX, f64::MAX),
                    },
                    meta.clone(),
                )
                .is_err()
        );
        let mut src_ovf = sample_clip("c_src_ovf", 0.0, f64::MAX);
        src_ovf.source_in_sec = f64::MAX;
        assert!(
            engine
                .apply(
                    TimelineCommand::PlaceClip {
                        track_id: "v1".into(),
                        clip: src_ovf,
                    },
                    meta.clone(),
                )
                .is_err()
        );
        assert!(
            engine
                .apply(
                    TimelineCommand::PlaceGfx {
                        track_id: "g1".into(),
                        clip_id: "g_ovf".into(),
                        label: "Overflow".into(),
                        start_sec: f64::MAX,
                        duration_sec: f64::MAX,
                        gfx: GfxInstance {
                            asset_id: None,
                            props: serde_json::json!({}),
                            inline_html: None,
                        },
                    },
                    meta.clone(),
                )
                .is_err()
        );

        // 4. RemoveMedia removes media asset and referencing clips atomically & supports undo
        engine
            .apply(
                TimelineCommand::AddMedia {
                    asset: crate::model::MediaAsset {
                        id: "m1".into(),
                        name: "draft.mp4".into(),
                        kind: crate::model::MediaKind::Video,
                        role_hint: "a_roll".into(),
                        path: "/tmp/draft.mp4".into(),
                        added_at: Utc::now(),
                        intelligence: MediaIntelligenceManifest::default(),
                    },
                },
                meta.clone(),
            )
            .expect("add m1");
        engine
            .apply(
                TimelineCommand::PlaceClip {
                    track_id: "v1".into(),
                    clip: sample_clip("c_ok", 0.0, 4.0),
                },
                meta.clone(),
            )
            .expect("place c_ok");
        assert!(
            engine
                .apply(
                    TimelineCommand::TrimClip {
                        track_id: "v1".into(),
                        clip_id: "c_ok".into(),
                        new_start_sec: Some(f64::MAX),
                        new_duration_sec: Some(f64::MAX),
                        new_source_in_sec: None,
                    },
                    meta.clone(),
                )
                .is_err()
        );

        let (res, _ev) = engine
            .apply(
                TimelineCommand::RemoveMedia {
                    media_id: "m1".into(),
                },
                meta.clone(),
            )
            .expect("remove media");
        assert_eq!(res.ui_anchor, "media.pool");
        assert!(engine.project().media_pool.is_empty());
        assert!(
            engine
                .project()
                .tracks
                .iter()
                .find(|t| t.id == "v1")
                .unwrap()
                .clips
                .is_empty()
        );

        engine.undo().expect("undo remove media");
        assert_eq!(engine.project().media_pool.len(), 1);
        assert_eq!(
            engine
                .project()
                .tracks
                .iter()
                .find(|t| t.id == "v1")
                .unwrap()
                .clips
                .len(),
            1
        );

        assert!(matches!(
            engine.apply(
                TimelineCommand::RemoveMedia {
                    media_id: "missing".into(),
                },
                meta,
            ),
            Err(CoreError::MediaNotFound(_))
        ));
    }

    #[test]
    fn batch_command_is_atomic_single_undo_and_checks_locks() {
        let mut engine = ProjectEngine::new(Project::new_default(
            "Batch Test",
            AspectPreset::Vertical9x16,
        ));
        let meta = CommandMeta::default();

        // Place 2 clips in 1 atomic batch
        let (res, _ev) = engine
            .apply(
                TimelineCommand::Batch {
                    commands: vec![
                        TimelineCommand::PlaceClip {
                            track_id: "v1".into(),
                            clip: sample_clip("b1", 0.0, 2.0),
                        },
                        TimelineCommand::PlaceClip {
                            track_id: "v1".into(),
                            clip: sample_clip("b2", 2.0, 3.0),
                        },
                    ],
                    summary: Some("placed 2 clips".into()),
                },
                meta.clone(),
            )
            .expect("batch place succeeds");
        assert_eq!(res.revision, 1);
        assert_eq!(res.summary, "placed 2 clips");
        assert_eq!(
            engine
                .project()
                .tracks
                .iter()
                .find(|t| t.id == "v1")
                .unwrap()
                .clips
                .len(),
            2
        );

        // Failing 2nd sub-command rolls back 1st sub-command completely
        let err = engine.apply(
            TimelineCommand::Batch {
                commands: vec![
                    TimelineCommand::DeleteClip {
                        track_id: "v1".into(),
                        clip_id: "b1".into(),
                    },
                    TimelineCommand::DeleteClip {
                        track_id: "v1".into(),
                        clip_id: "does-not-exist".into(),
                    },
                ],
                summary: None,
            },
            meta.clone(),
        );
        assert!(err.is_err());
        assert_eq!(engine.project().revision, 1);
        assert_eq!(
            engine
                .project()
                .tracks
                .iter()
                .find(|t| t.id == "v1")
                .unwrap()
                .clips
                .len(),
            2,
            "failed batch must roll back partial mutations"
        );

        // Locked track blocks batch touching it
        engine
            .apply(
                TimelineCommand::SetTrackFlags {
                    track_id: "v1".into(),
                    muted: None,
                    solo: None,
                    locked: Some(true),
                },
                meta.clone(),
            )
            .unwrap();
        let locked_err = engine.apply(
            TimelineCommand::Batch {
                commands: vec![TimelineCommand::DeleteClip {
                    track_id: "v1".into(),
                    clip_id: "b1".into(),
                }],
                summary: None,
            },
            meta.clone(),
        );
        assert!(matches!(locked_err, Err(CoreError::TrackLocked { .. })));

        // Unlock, batch move both clips, and undo in 1 step
        engine
            .apply(
                TimelineCommand::SetTrackFlags {
                    track_id: "v1".into(),
                    muted: None,
                    solo: None,
                    locked: Some(false),
                },
                meta.clone(),
            )
            .unwrap();
        engine
            .apply(
                TimelineCommand::Batch {
                    commands: vec![
                        TimelineCommand::TrimClip {
                            track_id: "v1".into(),
                            clip_id: "b1".into(),
                            new_start_sec: Some(1.0),
                            new_duration_sec: None,
                            new_source_in_sec: None,
                        },
                        TimelineCommand::TrimClip {
                            track_id: "v1".into(),
                            clip_id: "b2".into(),
                            new_start_sec: Some(3.0),
                            new_duration_sec: None,
                            new_source_in_sec: None,
                        },
                    ],
                    summary: Some("moved 2 clips".into()),
                },
                meta,
            )
            .expect("batch move succeeds");
        let v1_clips = &engine
            .project()
            .tracks
            .iter()
            .find(|t| t.id == "v1")
            .unwrap()
            .clips;
        assert!((v1_clips[0].start_sec - 1.0).abs() < 1e-6);
        assert!((v1_clips[1].start_sec - 3.0).abs() < 1e-6);

        engine.undo().expect("undo batch move");
        let v1_clips = &engine
            .project()
            .tracks
            .iter()
            .find(|t| t.id == "v1")
            .unwrap()
            .clips;
        assert!((v1_clips[0].start_sec - 0.0).abs() < 1e-6);
        assert!((v1_clips[1].start_sec - 2.0).abs() < 1e-6);
    }

    #[test]
    fn color_grade_brightness_defaults_zero_and_set_canvas_update_media_role_undo_cleanly() {
        let old_json = r#"{"lift":{"r":0,"g":0,"b":0},"gamma":{"r":0,"g":0,"b":0},"gain":{"r":0,"g":0,"b":0},"temp":0,"tint":0,"contrast":1,"saturation":1,"exposure":0,"lut_path":null,"lut_intensity":0.85}"#;
        let parsed: ColorGrade = serde_json::from_str(old_json).expect("backward compat serde");
        assert!((parsed.brightness - 0.0).abs() < 1e-9);

        let mut engine =
            ProjectEngine::new(Project::new_default("canvas", AspectPreset::Vertical9x16));
        let meta = CommandMeta::default();
        engine
            .apply(
                TimelineCommand::AddMedia {
                    asset: crate::model::MediaAsset {
                        id: "m1".into(),
                        name: "clip.mp4".into(),
                        kind: crate::model::MediaKind::Video,
                        role_hint: "a_roll".into(),
                        path: "/tmp/clip.mp4".into(),
                        added_at: chrono::Utc::now(),
                        intelligence: Default::default(),
                    },
                },
                meta.clone(),
            )
            .unwrap();

        engine
            .apply(
                TimelineCommand::SetCanvas {
                    canvas: crate::model::CanvasConfig::from_preset(AspectPreset::Landscape16x9),
                },
                meta.clone(),
            )
            .expect("set_canvas");
        assert_eq!(engine.project().canvas.width, 1920);
        assert_eq!(engine.project().canvas.height, 1080);

        engine
            .apply(
                TimelineCommand::UpdateMediaRole {
                    media_id: "m1".into(),
                    role_hint: "b_roll".into(),
                },
                meta,
            )
            .expect("update_media_role");
        assert_eq!(engine.project().media_pool[0].role_hint, "b_roll");

        engine.undo().unwrap();
        assert_eq!(engine.project().media_pool[0].role_hint, "a_roll");
        engine.undo().unwrap();
        assert_eq!(engine.project().canvas.width, 1080);
        assert_eq!(engine.project().canvas.height, 1920);
    }

    #[test]
    fn trim_clip_shifts_caption_word_timestamps_with_start_sec() {
        let mut engine =
            ProjectEngine::new(Project::new_default("cap_trim", AspectPreset::Vertical9x16));
        let meta = CommandMeta::default();
        let cap_clip = Clip {
            id: "c_cap".into(),
            label: "Hello world".into(),
            start_sec: 1.0,
            duration_sec: 2.0,
            source_in_sec: 0.0,
            transform: Transform2D::default(),
            audio: AudioSettings::default(),
            payload: ClipKind::Caption {
                caption: crate::model::CaptionBlock {
                    words: vec![
                        crate::model::CaptionWord {
                            text: "Hello".into(),
                            start_sec: 1.1,
                            end_sec: 1.5,
                        },
                        crate::model::CaptionWord {
                            text: "world".into(),
                            start_sec: 1.6,
                            end_sec: 2.2,
                        },
                    ],
                    preset: crate::model::CaptionPreset::NeonGlow,
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

        engine
            .apply(
                TimelineCommand::TrimClip {
                    track_id: "t1".into(),
                    clip_id: "c_cap".into(),
                    new_start_sec: Some(3.5),
                    new_duration_sec: None,
                    new_source_in_sec: None,
                },
                meta,
            )
            .unwrap();

        let t1 = engine
            .project()
            .tracks
            .iter()
            .find(|t| t.id == "t1")
            .unwrap();
        let ClipKind::Caption { caption } = &t1.clips[0].payload else {
            panic!("expected caption");
        };
        assert!((caption.words[0].start_sec - 3.6).abs() < 1e-6);
        assert!((caption.words[0].end_sec - 4.0).abs() < 1e-6);
        assert!((caption.words[1].start_sec - 4.1).abs() < 1e-6);
        assert!((caption.words[1].end_sec - 4.7).abs() < 1e-6);
    }
}
