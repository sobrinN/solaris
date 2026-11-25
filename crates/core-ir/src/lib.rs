pub mod commands;
pub mod engine;
pub mod error;
pub mod model;
pub mod registry;
pub mod templates;

pub use commands::{
    AgentRole, CommandMeta, EngineEvent, EngineEventEnvelope, OpResult, TimelineCommand,
};
pub use engine::{ProjectEngine, TrackLock, load_project, save_project_atomic};
pub use error::CoreError;
pub use model::{
    AspectPreset, AudioSettings, BrandPlaybook, BrandReferenceAsset, BrandReferenceKind,
    BrandTokens, CanvasConfig, CaptionBlock, CaptionPreset, CaptionWord, Clip, ClipKind,
    ColorGrade, ExportFormat, ExportJobStatus, ExportQuality, ExportRequest, ExportState,
    GfxInstance, LogEntry, LogLevel, MediaAsset, MediaIntelligenceManifest, MediaKind, OpLogEntry,
    Project, QaCheckItem, QaReport, QaSeverity, RgbTriplet, SavedCodeAsset, SmartReframeBox,
    SwarmRunState, SwarmRunStatus, TimeSpan, Track, TrackKind, TranscriptWord, Transform2D,
};
pub use registry::{
    CodeAssetPackManifest, MAX_GFX_HTML_BYTES, SOLARIS_ASSET_PACK_SCHEMA_VERSION,
    build_asset_pack_manifest, validate_asset_pack_manifest, validate_single_asset,
};
pub use templates::{ProjectTemplateInfo, builtin_templates, get_template, instantiate_template};
