pub mod analyze;
pub mod demo_media;
pub mod error;
pub mod export;
pub mod ffmpeg;
pub mod frames;
pub mod gfx_render;
pub mod lut;
pub mod probe;
pub mod s3_storage;
pub mod storage;
pub mod transcribe;

pub use analyze::{
    analyze_media_file, compute_peaks_from_f32_samples, detect_beats_from_f32_samples,
    detect_scene_cuts, detect_silences, estimate_default_reframe, extract_waveform_and_beats,
    extract_waveform_peaks, generate_llm_video_proxy, generate_proxy, generate_proxy_with_encoders,
    measure_loudness, parse_ebur128_integrated_lufs, parse_scdet_stderr,
    parse_silencedetect_stderr, probe_and_analyze_fast,
};
pub use demo_media::{DemoMediaFile, DemoMediaPack, ensure_demo_media_pack};
pub use error::MediaError;
pub use export::{
    ExportFfmpegPlan, ExportPlanOptions, append_color_grade_filters,
    build_color_grade_core_filters, build_export_plan, build_export_plan_with_options,
    escape_ffmpeg_filter_path, execute_export, execute_export_with_progress,
};
pub use ffmpeg::{MediaToolchain, ToolchainCapabilities, detect_chromium_binary};
pub use frames::{
    CompositeFrameOptions, build_grade_preview_vf, capture_composited_frame_png, capture_frame_png,
    capture_graded_frame_png, capture_overlay_only_frame_png, extract_keyframes_png,
};
pub use gfx_render::{
    BrandFontData, BrandFontFace, OverlayImageSpec, OverlayRenderProgress,
    build_caption_overlay_html, build_overlay_html, path_to_file_uri,
    render_project_overlays_via_chromium, render_project_overlays_via_chromium_with_assets,
    render_project_overlays_via_chromium_with_progress, render_single_frame_overlays_via_chromium,
};
pub use lut::{ParsedCubeLut, parse_cube_file, parse_cube_str};
pub use probe::{ProbeSummary, parse_ffprobe_json, probe_media};
pub use s3_storage::{S3Storage, S3StorageConfig};
pub use storage::{AssetStorage, LocalFsStorage};
pub use transcribe::{
    extract_audio_for_stt, parse_whisper_json, transcribe_via_ffmpeg_whisper,
    transcribe_via_ffmpeg_whisper_checked, validate_language_code,
};
