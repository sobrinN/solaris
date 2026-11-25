use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AspectPreset {
    Vertical9x16,
    Landscape16x9,
    Square1x1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasConfig {
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub aspect_preset: AspectPreset,
}

impl CanvasConfig {
    pub fn from_preset(preset: AspectPreset) -> Self {
        let (width, height) = match preset {
            AspectPreset::Vertical9x16 => (1080, 1920),
            AspectPreset::Landscape16x9 => (1920, 1080),
            AspectPreset::Square1x1 => (1080, 1080),
        };
        Self {
            width,
            height,
            fps_num: 30,
            fps_den: 1,
            aspect_preset: preset,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Text,
    Gfx,
    BRoll,
    ARoll,
    Grade,
    Voice,
    Music,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: String,
    pub code: String,
    pub name: String,
    pub subtitle: String,
    pub kind: TrackKind,
    pub muted: bool,
    pub solo: bool,
    pub locked: bool,
    pub clips: Vec<Clip>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SmartReframeBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transform2D {
    pub x: f64,
    pub y: f64,
    pub scale: f64,
    pub rotation_deg: f64,
    pub opacity: f64,
    #[serde(default)]
    pub smart_reframe: Option<SmartReframeBox>,
}

impl Default for Transform2D {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            scale: 1.0,
            rotation_deg: 0.0,
            opacity: 1.0,
            smart_reframe: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RgbTriplet {
    pub r: f64,
    pub g: f64,
    pub b: f64,
}

impl Default for RgbTriplet {
    fn default() -> Self {
        Self {
            r: 0.0,
            g: 0.0,
            b: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColorGrade {
    pub lift: RgbTriplet,
    pub gamma: RgbTriplet,
    pub gain: RgbTriplet,
    pub temp: f64,
    pub tint: f64,
    pub contrast: f64,
    pub saturation: f64,
    pub exposure: f64,
    #[serde(default)]
    pub lut_path: Option<String>,
    pub lut_intensity: f64,
}

impl Default for ColorGrade {
    fn default() -> Self {
        Self {
            lift: RgbTriplet::default(),
            gamma: RgbTriplet::default(),
            gain: RgbTriplet::default(),
            temp: 0.0,
            tint: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            exposure: 0.0,
            lut_path: None,
            lut_intensity: 0.85,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptionPreset {
    NeonGlow,
    MonoCaps,
    Karaoke,
    Minimal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptionWord {
    pub text: String,
    pub start_sec: f64,
    pub end_sec: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptionBlock {
    pub words: Vec<CaptionWord>,
    pub preset: CaptionPreset,
    pub font_family: String,
    pub highlight_color: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GfxInstance {
    #[serde(default)]
    pub asset_id: Option<String>,
    pub props: serde_json::Value,
    #[serde(default)]
    pub inline_html: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioSettings {
    pub gain_db: f64,
    pub ducking_enabled: bool,
    pub duck_target_db: f64,
    pub fade_in_sec: f64,
    pub fade_out_sec: f64,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            gain_db: 0.0,
            ducking_enabled: false,
            duck_target_db: -18.0,
            fade_in_sec: 0.0,
            fade_out_sec: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClipKind {
    Media {
        media_id: String,
        #[serde(default)]
        ken_burns: bool,
    },
    Caption {
        caption: CaptionBlock,
    },
    Gfx {
        gfx: GfxInstance,
    },
    Grade {
        grade: ColorGrade,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: String,
    pub label: String,
    pub start_sec: f64,
    pub duration_sec: f64,
    pub source_in_sec: f64,
    pub transform: Transform2D,
    pub audio: AudioSettings,
    pub payload: ClipKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Video,
    Image,
    Audio,
    Document,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeSpan {
    pub start_sec: f64,
    pub end_sec: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptWord {
    pub word: String,
    pub start_sec: f64,
    pub end_sec: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MediaIntelligenceManifest {
    pub analyzed: bool,
    pub duration_sec: f64,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub fps: Option<f64>,
    #[serde(default)]
    pub codec: Option<String>,
    #[serde(default)]
    pub color_space: Option<String>,
    pub is_log_footage: bool,
    #[serde(default)]
    pub waveform_peaks: Vec<f32>,
    #[serde(default)]
    pub silences: Vec<TimeSpan>,
    #[serde(default)]
    pub beats: Vec<f64>,
    #[serde(default)]
    pub scene_cuts: Vec<f64>,
    #[serde(default)]
    pub default_reframe: Option<SmartReframeBox>,
    #[serde(default)]
    pub transcript: Vec<TranscriptWord>,
    #[serde(default)]
    pub integrated_lufs: Option<f64>,
    #[serde(default)]
    pub proxy_path: Option<String>,
    #[serde(default)]
    pub analysis_error: Option<String>,
    #[serde(default)]
    pub has_audio: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaAsset {
    pub id: String,
    pub name: String,
    pub kind: MediaKind,
    pub role_hint: String,
    pub path: String,
    pub added_at: DateTime<Utc>,
    pub intelligence: MediaIntelligenceManifest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrandTokens {
    pub bg: String,
    pub surface: String,
    pub ink: String,
    pub accent: String,
    pub accent_secondary: String,
    pub font_display: String,
    pub font_body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrandReferenceKind {
    ReferenceVideo,
    StyleFrame,
    Logo,
    AudioIdentity,
    LutCube,
    Font,
    Document,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrandReferenceAsset {
    pub id: String,
    pub name: String,
    pub kind: BrandReferenceKind,
    pub path: String,
    #[serde(default)]
    pub notes: String,
    pub added_at: DateTime<Utc>,
    #[serde(default)]
    pub intelligence: MediaIntelligenceManifest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrandPlaybook {
    pub id: String,
    pub name: String,
    pub brand_md: String,
    pub tokens: BrandTokens,
    #[serde(default)]
    pub default_lut: Option<String>,
    pub pacing_rules: String,
    #[serde(default)]
    pub reference_media: Vec<BrandReferenceAsset>,
    pub updated_at: DateTime<Utc>,
}

impl BrandPlaybook {
    pub fn solaris_default() -> Self {
        Self {
            id: "solaris-default".to_string(),
            name: "Solaris Editorial Default".to_string(),
            brand_md: "# Brand Playbook\n\n- Hook in the first 2.5 seconds with bold motion or a punch-in.\n- Keep captions inside the 80% social title-safe zone (2-4 words per burst).\n- Target -14 LUFS integrated loudness with -18 dB music ducking under speech.\n".to_string(),
            tokens: BrandTokens {
                bg: "#080404".to_string(),
                surface: "#1A0E0E".to_string(),
                ink: "#F7F5EF".to_string(),
                accent: "#F05A00".to_string(),
                accent_secondary: "#F7B733".to_string(),
                font_display: "Sora".to_string(),
                font_body: "Sora".to_string(),
            },
            default_lut: None,
            pacing_rules: "Cut silences > 0.25s with 0.12s breath pad; B-roll cutaway every 3.5s.".to_string(),
            reference_media: Vec::new(),
            updated_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpLogEntry {
    pub revision: u64,
    pub timestamp: DateTime<Utc>,
    pub agent_id: String,
    pub role: crate::commands::AgentRole,
    pub command_kind: String,
    pub summary: String,
    pub ui_anchor: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Mp4,
    MovProRes4444,
    WebmAlpha,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportQuality {
    Draft,
    Standard,
    High,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportRequest {
    pub format: ExportFormat,
    pub quality: ExportQuality,
    #[serde(default = "default_target_lufs")]
    pub target_lufs: f64,
    #[serde(default)]
    pub output_name: Option<String>,
}

fn default_target_lufs() -> f64 {
    -14.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportState {
    Queued,
    RenderingGfx,
    Compositing,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportJobStatus {
    pub id: String,
    pub project_id: String,
    pub state: ExportState,
    pub progress: f32,
    pub stage_label: String,
    #[serde(default)]
    pub output_path: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedCodeAsset {
    pub id: String,
    pub name: String,
    pub category: String,
    pub tags: Vec<String>,
    pub props_schema: serde_json::Value,
    pub default_props: serde_json::Value,
    pub html_source: String,
    pub default_duration_sec: f64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QaSeverity {
    Pass,
    Warning,
    Fixed,
    Issue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QaCheckItem {
    pub id: String,
    pub category: String,
    pub severity: QaSeverity,
    pub message: String,
    #[serde(default)]
    pub time_sec: Option<f64>,
    pub auto_fixed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QaReport {
    pub passed: bool,
    pub score: u8,
    pub frames_inspected: usize,
    pub auto_fixes_applied: u32,
    pub checks: Vec<QaCheckItem>,
    #[serde(default)]
    pub multimodal_summary: Option<String>,
    pub checked_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwarmRunState {
    Queued,
    Planning,
    ExecutingSpine,
    ExecutingSpecialists,
    RunningQa,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SwarmRunStatus {
    pub run_id: String,
    pub project_id: String,
    pub state: SwarmRunState,
    pub phase_label: String,
    #[serde(default)]
    pub model_used: Option<String>,
    pub used_local_fallback: bool,
    #[serde(default)]
    pub active_roles: Vec<crate::commands::AgentRole>,
    pub ops_committed: u32,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub qa_report: Option<QaReport>,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub level: LogLevel,
    pub source: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    pub message: String,
    #[serde(default)]
    pub details: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub prompt: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub revision: u64,
    pub canvas: CanvasConfig,
    pub brand_playbook_id: String,
    pub media_pool: Vec<MediaAsset>,
    pub tracks: Vec<Track>,
    pub active_grade: ColorGrade,
    #[serde(default)]
    pub activity_log: Vec<OpLogEntry>,
}

impl Project {
    pub fn new_default(name: impl Into<String>, aspect: AspectPreset) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            prompt: String::new(),
            created_at: now,
            updated_at: now,
            revision: 0,
            canvas: CanvasConfig::from_preset(aspect),
            brand_playbook_id: "solaris-default".to_string(),
            media_pool: Vec::new(),
            tracks: default_tracks(),
            active_grade: ColorGrade::default(),
            activity_log: Vec::new(),
        }
    }

    pub fn duration_sec(&self) -> f64 {
        self.tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .map(|c| c.start_sec + c.duration_sec)
            .fold(0.0_f64, f64::max)
    }
}

pub fn default_tracks() -> Vec<Track> {
    vec![
        Track {
            id: "t1".to_string(),
            code: "T1".to_string(),
            name: "TEXT".to_string(),
            subtitle: "captions".to_string(),
            kind: TrackKind::Text,
            muted: false,
            solo: false,
            locked: false,
            clips: Vec::new(),
        },
        Track {
            id: "g1".to_string(),
            code: "G1".to_string(),
            name: "GFX".to_string(),
            subtitle: "overlays".to_string(),
            kind: TrackKind::Gfx,
            muted: false,
            solo: false,
            locked: false,
            clips: Vec::new(),
        },
        Track {
            id: "v2".to_string(),
            code: "V2".to_string(),
            name: "B-ROLL".to_string(),
            subtitle: "inserts".to_string(),
            kind: TrackKind::BRoll,
            muted: false,
            solo: false,
            locked: false,
            clips: Vec::new(),
        },
        Track {
            id: "v1".to_string(),
            code: "V1".to_string(),
            name: "A-ROLL".to_string(),
            subtitle: "primary".to_string(),
            kind: TrackKind::ARoll,
            muted: false,
            solo: false,
            locked: false,
            clips: Vec::new(),
        },
        Track {
            id: "fx".to_string(),
            code: "FX".to_string(),
            name: "GRADE".to_string(),
            subtitle: "adjustment".to_string(),
            kind: TrackKind::Grade,
            muted: false,
            solo: false,
            locked: false,
            clips: Vec::new(),
        },
        Track {
            id: "a1".to_string(),
            code: "A1".to_string(),
            name: "VOICE".to_string(),
            subtitle: "dialog".to_string(),
            kind: TrackKind::Voice,
            muted: false,
            solo: false,
            locked: false,
            clips: Vec::new(),
        },
        Track {
            id: "a2".to_string(),
            code: "A2".to_string(),
            name: "MUSIC".to_string(),
            subtitle: "beat + sfx".to_string(),
            kind: TrackKind::Music,
            muted: false,
            solo: false,
            locked: false,
            clips: Vec::new(),
        },
    ]
}
