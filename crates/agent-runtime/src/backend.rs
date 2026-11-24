use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};
use solaris_core_ir::{
    AgentRole, AspectPreset, BrandPlaybook, CommandMeta, ExportJobStatus, ExportRequest,
    MediaAsset, OpResult, Project, SavedCodeAsset, TimelineCommand, TranscriptWord,
};

use crate::error::AgentError;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyframeResult {
    pub time_sec: f64,
    pub png_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapturedFrame {
    pub time_sec: f64,
    pub width: u32,
    pub height: u32,
    pub png_base64: String,
    pub note: String,
}

pub trait SolarisBackend: Send + Sync {
    fn list_projects(&self) -> BoxFuture<'_, Result<Vec<serde_json::Value>, AgentError>>;

    fn create_project(
        &self,
        name: String,
        aspect_preset: Option<AspectPreset>,
        brand_playbook_id: Option<String>,
        prompt: Option<String>,
        template_id: Option<String>,
    ) -> BoxFuture<'_, Result<Project, AgentError>>;

    fn get_project(&self, project_id: &str) -> BoxFuture<'_, Result<Project, AgentError>>;

    fn apply_op(
        &self,
        project_id: &str,
        command: TimelineCommand,
        meta: CommandMeta,
    ) -> BoxFuture<'_, Result<(OpResult, Project), AgentError>>;

    fn undo(&self, project_id: &str) -> BoxFuture<'_, Result<Project, AgentError>>;

    fn redo(&self, project_id: &str) -> BoxFuture<'_, Result<Project, AgentError>>;

    fn acquire_track_lock(
        &self,
        project_id: &str,
        track_id: &str,
        agent_id: &str,
        role: AgentRole,
    ) -> BoxFuture<'_, Result<(), AgentError>>;

    fn release_track_lock(
        &self,
        project_id: &str,
        track_id: &str,
        agent_id: &str,
    ) -> BoxFuture<'_, Result<(), AgentError>>;

    fn import_media_paths(
        &self,
        project_id: &str,
        paths: Vec<(String, Option<String>)>,
    ) -> BoxFuture<'_, Result<Vec<MediaAsset>, AgentError>>;

    fn transcribe_media(
        &self,
        project_id: &str,
        media_id: &str,
        language: Option<String>,
    ) -> BoxFuture<'_, Result<Vec<TranscriptWord>, AgentError>>;

    fn extract_keyframes(
        &self,
        project_id: &str,
        media_id: &str,
        timestamps_sec: Vec<f64>,
    ) -> BoxFuture<'_, Result<Vec<KeyframeResult>, AgentError>>;

    fn capture_preview_frame(
        &self,
        project_id: &str,
        time_sec: f64,
    ) -> BoxFuture<'_, Result<CapturedFrame, AgentError>>;

    fn list_brands(&self) -> BoxFuture<'_, Result<Vec<BrandPlaybook>, AgentError>>;

    fn get_brand(&self, brand_id: &str) -> BoxFuture<'_, Result<BrandPlaybook, AgentError>>;

    fn list_assets(&self) -> BoxFuture<'_, Result<Vec<SavedCodeAsset>, AgentError>>;

    fn save_asset(
        &self,
        asset: SavedCodeAsset,
    ) -> BoxFuture<'_, Result<SavedCodeAsset, AgentError>>;

    fn start_export(
        &self,
        project_id: &str,
        req: ExportRequest,
    ) -> BoxFuture<'_, Result<ExportJobStatus, AgentError>>;

    fn emit_cursor_pulse(
        &self,
        project_id: &str,
        agent_id: &str,
        role: AgentRole,
        ui_anchor: &str,
        label: &str,
    ) -> BoxFuture<'_, Result<(), AgentError>>;

    fn emit_playhead_seek(
        &self,
        project_id: &str,
        time_sec: f64,
        agent_id: &str,
        role: AgentRole,
    ) -> BoxFuture<'_, Result<(), AgentError>>;

    /// Read a bounded text excerpt from a `MediaKind::Document` file (e.g. `.md`, `.txt`, `.pdf`).
    fn read_document_excerpt(
        &self,
        _project_id: &str,
        _media_id: &str,
        _max_chars: usize,
    ) -> BoxFuture<'_, Result<Option<String>, AgentError>> {
        Box::pin(async { Ok(None) })
    }

    /// Generate a compact 480p low-FPS base64 MP4 video proxy within `max_bytes` for native omni-modal LLMs.
    fn prepare_llm_video_proxy(
        &self,
        _project_id: &str,
        _media_id: &str,
        _max_bytes: usize,
    ) -> BoxFuture<'_, Result<Option<String>, AgentError>> {
        Box::pin(async { Ok(None) })
    }

    /// Read a compact resized base64 image `(mime_type, base64_data)` for a `MediaKind::Image` asset.
    fn read_media_image_base64(
        &self,
        _project_id: &str,
        _media_id: &str,
    ) -> BoxFuture<'_, Result<Option<(String, String)>, AgentError>> {
        Box::pin(async { Ok(None) })
    }

    /// Read a compact base64 audio clip `(format, base64_data)` up to `max_sec` for a voice/audio asset.
    fn read_media_audio_base64(
        &self,
        _project_id: &str,
        _media_id: &str,
        _max_sec: f64,
    ) -> BoxFuture<'_, Result<Option<(String, String)>, AgentError>> {
        Box::pin(async { Ok(None) })
    }

    /// Best-effort non-blocking track lock release used by RAII drop guards holding `&dyn SolarisBackend`.
    fn spawn_release_track_lock(&self, _project_id: &str, _track_id: &str, _agent_id: &str) {}
}
