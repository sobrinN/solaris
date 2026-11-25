use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::{
    AudioSettings, CaptionBlock, Clip, ColorGrade, GfxInstance, MediaAsset,
    MediaIntelligenceManifest, Project, Transform2D,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    User,
    Director,
    Razor,
    BRoll,
    Grade,
    Caps,
    Gfx,
    Mix,
    McpExternal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandMeta {
    pub agent_id: String,
    pub role: AgentRole,
    #[serde(default)]
    pub reason: Option<String>,
}

impl Default for CommandMeta {
    fn default() -> Self {
        Self {
            agent_id: "user".to_string(),
            role: AgentRole::User,
            reason: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimelineCommand {
    AddMedia {
        asset: MediaAsset,
    },
    RemoveMedia {
        media_id: String,
    },
    UpdateMediaIntelligence {
        media_id: String,
        intelligence: MediaIntelligenceManifest,
    },
    PlaceClip {
        track_id: String,
        clip: Clip,
    },
    SplitClip {
        track_id: String,
        clip_id: String,
        split_at_timeline_sec: f64,
        right_clip_id: String,
    },
    TrimClip {
        track_id: String,
        clip_id: String,
        #[serde(default)]
        new_start_sec: Option<f64>,
        #[serde(default)]
        new_duration_sec: Option<f64>,
        #[serde(default)]
        new_source_in_sec: Option<f64>,
    },
    DeleteClip {
        track_id: String,
        clip_id: String,
    },
    UpdateClipTransform {
        track_id: String,
        clip_id: String,
        transform: Transform2D,
    },
    SetColorGrade {
        grade: ColorGrade,
        #[serde(default)]
        track_id: Option<String>,
        #[serde(default)]
        clip_id: Option<String>,
    },
    UpdateCaptionClip {
        track_id: String,
        clip_id: String,
        caption: CaptionBlock,
    },
    PlaceGfx {
        track_id: String,
        clip_id: String,
        label: String,
        start_sec: f64,
        duration_sec: f64,
        gfx: GfxInstance,
    },
    UpdateGfxProps {
        track_id: String,
        clip_id: String,
        props: serde_json::Value,
    },
    SetAudioSettings {
        track_id: String,
        clip_id: String,
        audio: AudioSettings,
    },
    SetTrackFlags {
        track_id: String,
        #[serde(default)]
        muted: Option<bool>,
        #[serde(default)]
        solo: Option<bool>,
        #[serde(default)]
        locked: Option<bool>,
    },
    SetBrandPlaybook {
        brand_playbook_id: String,
    },
}

impl TimelineCommand {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::AddMedia { .. } => "add_media",
            Self::RemoveMedia { .. } => "remove_media",
            Self::UpdateMediaIntelligence { .. } => "update_media_intelligence",
            Self::PlaceClip { .. } => "place_clip",
            Self::SplitClip { .. } => "split_clip",
            Self::TrimClip { .. } => "trim_clip",
            Self::DeleteClip { .. } => "delete_clip",
            Self::UpdateClipTransform { .. } => "update_clip_transform",
            Self::SetColorGrade { .. } => "set_color_grade",
            Self::UpdateCaptionClip { .. } => "update_caption_clip",
            Self::PlaceGfx { .. } => "place_gfx",
            Self::UpdateGfxProps { .. } => "update_gfx_props",
            Self::SetAudioSettings { .. } => "set_audio_settings",
            Self::SetTrackFlags { .. } => "set_track_flags",
            Self::SetBrandPlaybook { .. } => "set_brand_playbook",
        }
    }

    pub fn target_track_id(&self) -> Option<&str> {
        match self {
            Self::PlaceClip { track_id, .. }
            | Self::SplitClip { track_id, .. }
            | Self::TrimClip { track_id, .. }
            | Self::DeleteClip { track_id, .. }
            | Self::UpdateClipTransform { track_id, .. }
            | Self::UpdateCaptionClip { track_id, .. }
            | Self::PlaceGfx { track_id, .. }
            | Self::UpdateGfxProps { track_id, .. }
            | Self::SetAudioSettings { track_id, .. }
            | Self::SetTrackFlags { track_id, .. } => Some(track_id.as_str()),
            Self::SetColorGrade { track_id, .. } => Some(track_id.as_deref().unwrap_or("fx")),
            Self::AddMedia { .. }
            | Self::RemoveMedia { .. }
            | Self::UpdateMediaIntelligence { .. }
            | Self::SetBrandPlaybook { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpResult {
    pub revision: u64,
    pub summary: String,
    pub ui_anchor: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum EngineEvent {
    OpCommitted {
        project_id: String,
        revision: u64,
        agent_id: String,
        role: AgentRole,
        command_kind: String,
        summary: String,
        ui_anchor: String,
        project: Box<Project>,
    },
    ProjectReset {
        project_id: String,
        revision: u64,
        reason: String,
        project: Box<Project>,
    },
    MediaAnalyzed {
        project_id: String,
        media_id: String,
        intelligence: crate::model::MediaIntelligenceManifest,
    },
    TrackLockChanged {
        project_id: String,
        track_id: String,
        holder_agent_id: Option<String>,
        role: Option<AgentRole>,
    },
    CursorPulse {
        project_id: String,
        agent_id: String,
        role: AgentRole,
        ui_anchor: String,
        label: String,
    },
    ExportProgress {
        project_id: String,
        job: crate::model::ExportJobStatus,
    },
    SwarmProgress {
        project_id: String,
        run: crate::model::SwarmRunStatus,
    },
    PlayheadSeek {
        project_id: String,
        time_sec: f64,
        agent_id: String,
        role: AgentRole,
    },
    LogEmitted {
        #[serde(default)]
        project_id: Option<String>,
        entry: crate::model::LogEntry,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineEventEnvelope {
    pub timestamp: DateTime<Utc>,
    pub payload: EngineEvent,
}

impl EngineEventEnvelope {
    pub fn now(payload: EngineEvent) -> Self {
        Self {
            timestamp: Utc::now(),
            payload,
        }
    }
}
