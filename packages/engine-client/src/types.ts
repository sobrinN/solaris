export type AspectPreset = "vertical9x16" | "landscape16x9" | "square1x1";

export type TrackKind =
  | "text"
  | "gfx"
  | "b_roll"
  | "a_roll"
  | "grade"
  | "voice"
  | "music";

export type AgentRole =
  | "user"
  | "director"
  | "razor"
  | "b_roll"
  | "grade"
  | "caps"
  | "gfx"
  | "mix"
  | "mcp_external";

export interface CanvasConfig {
  width: number;
  height: number;
  fps_num: number;
  fps_den: number;
  aspect_preset: AspectPreset;
}

export interface SmartReframeBox {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface Transform2D {
  x: number;
  y: number;
  scale: number;
  rotation_deg: number;
  opacity: number;
  smart_reframe?: SmartReframeBox | null;
}

export interface RgbTriplet {
  r: number;
  g: number;
  b: number;
}

export interface ColorGrade {
  lift: RgbTriplet;
  gamma: RgbTriplet;
  gain: RgbTriplet;
  temp: number;
  tint: number;
  contrast: number;
  saturation: number;
  exposure: number;
  lut_path?: string | null;
  lut_intensity: number;
}

export type CaptionPreset = "neon_glow" | "mono_caps" | "karaoke" | "minimal";

export interface CaptionWord {
  text: string;
  start_sec: number;
  end_sec: number;
}

export interface CaptionBlock {
  words: CaptionWord[];
  preset: CaptionPreset;
  font_family: string;
  highlight_color: string;
}

export interface GfxInstance {
  asset_id?: string | null;
  props: Record<string, unknown>;
  inline_html?: string | null;
}

export interface AudioSettings {
  gain_db: number;
  ducking_enabled: boolean;
  duck_target_db: number;
  fade_in_sec: number;
  fade_out_sec: number;
}

export type ClipKind =
  | { kind: "media"; media_id: string; ken_burns?: boolean }
  | { kind: "caption"; caption: CaptionBlock }
  | { kind: "gfx"; gfx: GfxInstance }
  | { kind: "grade"; grade: ColorGrade };

export interface Clip {
  id: string;
  label: string;
  start_sec: number;
  duration_sec: number;
  source_in_sec: number;
  transform: Transform2D;
  audio: AudioSettings;
  payload: ClipKind;
}

export interface Track {
  id: string;
  code: string;
  name: string;
  subtitle: string;
  kind: TrackKind;
  muted: boolean;
  solo: boolean;
  locked: boolean;
  clips: Clip[];
}

export type MediaKind = "video" | "image" | "audio" | "document";

export interface TimeSpan {
  start_sec: number;
  end_sec: number;
}

export interface TranscriptWord {
  word: string;
  start_sec: number;
  end_sec: number;
}

export interface MediaIntelligenceManifest {
  analyzed: boolean;
  duration_sec: number;
  width?: number | null;
  height?: number | null;
  fps?: number | null;
  codec?: string | null;
  color_space?: string | null;
  is_log_footage: boolean;
  waveform_peaks: number[];
  silences: TimeSpan[];
  beats: number[];
  scene_cuts?: number[];
  default_reframe?: SmartReframeBox | null;
  transcript: TranscriptWord[];
  integrated_lufs?: number | null;
  proxy_path?: string | null;
  analysis_error?: string | null;
  has_audio?: boolean;
}

export interface MediaAsset {
  id: string;
  name: string;
  kind: MediaKind;
  role_hint: string;
  path: string;
  added_at: string;
  intelligence: MediaIntelligenceManifest;
}

export interface BrandTokens {
  bg: string;
  surface: string;
  ink: string;
  accent: string;
  accent_secondary: string;
  font_display: string;
  font_body: string;
}

export type BrandReferenceKind =
  | "reference_video"
  | "style_frame"
  | "logo"
  | "audio_identity"
  | "lut_cube"
  | "font"
  | "document";

export interface BrandReferenceAsset {
  id: string;
  name: string;
  kind: BrandReferenceKind;
  path: string;
  notes: string;
  added_at: string;
  intelligence: MediaIntelligenceManifest;
}

export interface BrandPlaybook {
  id: string;
  name: string;
  brand_md: string;
  tokens: BrandTokens;
  default_lut?: string | null;
  pacing_rules: string;
  reference_media: BrandReferenceAsset[];
  updated_at: string;
}

export interface OpLogEntry {
  revision: number;
  timestamp: string;
  agent_id: string;
  role: AgentRole;
  command_kind: string;
  summary: string;
  ui_anchor: string;
}

export type ExportFormat = "mp4" | "mov_pro_res4444" | "webm_alpha";
export type ExportQuality = "draft" | "standard" | "high";

export type QaSeverity = "pass" | "warning" | "fixed" | "issue";

export interface QaCheckItem {
  id: string;
  category: string;
  severity: QaSeverity;
  message: string;
  time_sec?: number | null;
  auto_fixed: boolean;
}

export interface QaReport {
  passed: boolean;
  score: number;
  frames_inspected: number;
  auto_fixes_applied: number;
  checks: QaCheckItem[];
  multimodal_summary?: string | null;
  checked_at: string;
}

export interface ProjectTemplateInfo {
  id: string;
  name: string;
  category: string;
  description: string;
  aspect_preset: AspectPreset;
  recommended_prompt: string;
  default_duration_sec: number;
  tags: string[];
}

export interface CodeAssetPackManifest {
  schema_version: string;
  pack_id: string;
  name: string;
  version: string;
  author: string;
  license: string;
  description: string;
  assets: SavedCodeAsset[];
}

export interface AssetPackImportResponse {
  imported_count: number;
  assets: SavedCodeAsset[];
}

export type SwarmRunState =
  | "queued"
  | "planning"
  | "executing_spine"
  | "executing_specialists"
  | "running_qa"
  | "completed"
  | "failed"
  | "cancelled";

export interface SwarmRunStatus {
  run_id: string;
  project_id: string;
  state: SwarmRunState;
  phase_label: string;
  model_used?: string | null;
  used_local_fallback: boolean;
  active_roles: AgentRole[];
  ops_committed: number;
  summary?: string | null;
  error?: string | null;
  qa_report?: QaReport | null;
  started_at: string;
  updated_at: string;
}

export interface SwarmRunRequest {
  prompt: string;
  model_override?: string;
  allow_local_fallback?: boolean;
  max_turns_per_worker?: number;
  skip_qa?: boolean;
}

export interface AgentStatusResponse {
  openrouter_configured: boolean;
  default_model?: string | null;
  has_whisper_filter: boolean;
  mcp_http_path: string;
  skills_count: number;
}

export interface ModelCandidate {
  id: string;
  name: string;
  context_length: number;
  prompt_usd_per_m?: number | null;
  completion_usd_per_m?: number | null;
  input_modalities: string[];
  output_modalities: string[];
  supports_tools: boolean;
  score: number;
  match_tier?: "best_match" | "smart_fallback" | "text_only";
}

export interface ModelRecommendation {
  role: "director_multimodal" | "specialist_tool_worker" | "transcription_stt";
  primary: string;
  fallbacks: string[];
  candidates: ModelCandidate[];
  source: string;
  fetched_at: string;
}

export interface ModelAdvisorResponse {
  director: ModelRecommendation;
  worker: ModelRecommendation;
  stt: ModelRecommendation;
}

export interface SkillSummary {
  name: string;
  description: string;
  uri: string;
}

export interface ExportRequest {
  format: ExportFormat;
  quality: ExportQuality;
  target_lufs?: number;
  output_name?: string;
}

export type ExportState =
  | "queued"
  | "rendering_gfx"
  | "compositing"
  | "completed"
  | "failed";

export interface ExportJobStatus {
  id: string;
  project_id: string;
  state: ExportState;
  progress: number;
  stage_label: string;
  output_path?: string | null;
  error?: string | null;
  updated_at: string;
}

export interface ParsedCubeLut {
  title?: string | null;
  size: number;
  domain_min: [number, number, number];
  domain_max: [number, number, number];
  data: number[];
}

export interface SavedCodeAsset {
  id: string;
  name: string;
  category: string;
  tags: string[];
  props_schema: Record<string, unknown>;
  default_props: Record<string, unknown>;
  html_source: string;
  default_duration_sec: number;
  created_at: string;
}

export type LogLevel = "debug" | "info" | "warn" | "error";

export interface LogEntry {
  id: string;
  timestamp: string;
  level: LogLevel;
  source: string;
  project_id?: string | null;
  run_id?: string | null;
  agent_id?: string | null;
  message: string;
  details?: string | null;
}

export interface Project {
  id: string;
  name: string;
  prompt?: string;
  created_at: string;
  updated_at: string;
  revision: number;
  canvas: CanvasConfig;
  brand_playbook_id: string;
  media_pool: MediaAsset[];
  tracks: Track[];
  active_grade: ColorGrade;
  activity_log?: OpLogEntry[];
}

export interface ProjectSummary {
  id: string;
  name: string;
  revision: number;
  aspect_preset: AspectPreset;
  duration_sec: number;
  media_count: number;
  updated_at: string;
}

export interface ToolchainCapabilities {
  ffmpeg_available: boolean;
  ffprobe_available: boolean;
  chromium_available: boolean;
  ffmpeg_version?: string | null;
  hw_encoders: string[];
  has_whisper_filter: boolean;
}

export interface HealthResponse {
  status: string;
  version: string;
  storage_backend?: string;
  toolchain: ToolchainCapabilities;
}

export interface CommandMeta {
  agent_id: string;
  role: AgentRole;
  reason?: string;
}

export interface UpdateProjectMetadataRequest {
  name?: string;
  prompt?: string;
}

export interface GfxSandboxResponse {
  id: string;
  path: string;
}

export type TimelineCommand =
  | { type: "add_media"; asset: MediaAsset }
  | { type: "remove_media"; media_id: string }
  | {
      type: "update_media_intelligence";
      media_id: string;
      intelligence: MediaIntelligenceManifest;
    }
  | {
      type: "place_clip";
      track_id: string;
      clip: Clip;
    }
  | {
      type: "split_clip";
      track_id: string;
      clip_id: string;
      split_at_timeline_sec: number;
      right_clip_id: string;
    }
  | {
      type: "trim_clip";
      track_id: string;
      clip_id: string;
      new_start_sec?: number;
      new_duration_sec?: number;
      new_source_in_sec?: number;
    }
  | { type: "delete_clip"; track_id: string; clip_id: string }
  | {
      type: "update_clip_transform";
      track_id: string;
      clip_id: string;
      transform: Transform2D;
    }
  | {
      type: "set_color_grade";
      grade: ColorGrade;
      track_id?: string | null;
      clip_id?: string | null;
    }
  | {
      type: "update_caption_clip";
      track_id: string;
      clip_id: string;
      caption: CaptionBlock;
    }
  | {
      type: "place_gfx";
      track_id: string;
      clip_id: string;
      label: string;
      start_sec: number;
      duration_sec: number;
      gfx: GfxInstance;
    }
  | {
      type: "update_gfx_props";
      track_id: string;
      clip_id: string;
      props: Record<string, unknown>;
    }
  | {
      type: "set_audio_settings";
      track_id: string;
      clip_id: string;
      audio: AudioSettings;
    }
  | {
      type: "set_track_flags";
      track_id: string;
      muted?: boolean;
      solo?: boolean;
      locked?: boolean;
    }
  | { type: "set_brand_playbook"; brand_playbook_id: string };

export interface OpResult {
  revision: number;
  summary: string;
  ui_anchor: string;
}

export type EngineEvent =
  | {
      event: "op_committed";
      project_id: string;
      revision: number;
      agent_id: string;
      role: AgentRole;
      command_kind: string;
      summary: string;
      ui_anchor: string;
      project: Project;
    }
  | {
      event: "project_reset";
      project_id: string;
      revision: number;
      reason: string;
      project: Project;
    }
  | {
      event: "media_analyzed";
      project_id: string;
      media_id: string;
      intelligence: MediaIntelligenceManifest;
    }
  | {
      event: "track_lock_changed";
      project_id: string;
      track_id: string;
      holder_agent_id?: string | null;
      role?: AgentRole | null;
    }
  | {
      event: "cursor_pulse";
      project_id: string;
      agent_id: string;
      role: AgentRole;
      ui_anchor: string;
      label: string;
    }
  | {
      event: "export_progress";
      project_id: string;
      job: ExportJobStatus;
    }
  | {
      event: "swarm_progress";
      project_id: string;
      run: SwarmRunStatus;
    }
  | {
      event: "playhead_seek";
      project_id: string;
      time_sec: number;
      agent_id: string;
      role: AgentRole;
    }
  | {
      event: "log_emitted";
      project_id?: string | null;
      entry: LogEntry;
    };

export interface EngineEventEnvelope {
  timestamp: string;
  payload: EngineEvent;
}
