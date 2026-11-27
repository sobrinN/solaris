use serde::Serialize;
use serde_json::{Value, json};
use solaris_core_ir::AgentRole;

use crate::openrouter::{ToolDefinition, ToolFunctionDef};

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolAnnotations {
    pub read_only_hint: bool,
    pub destructive_hint: bool,
    pub idempotent_hint: bool,
    pub open_world_hint: bool,
}

#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub mcp_name: &'static str,
    pub llm_name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
    pub annotations: ToolAnnotations,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolOutput {
    pub text: String,
    pub structured: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_png_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images_png_base64: Vec<String>,
}

impl ToolOutput {
    pub fn all_images_base64(&self) -> Vec<&str> {
        let mut out = Vec::with_capacity(
            usize::from(self.image_png_base64.is_some()) + self.images_png_base64.len(),
        );
        if let Some(ref img) = self.image_png_base64 {
            out.push(img.as_str());
        }
        for img in &self.images_png_base64 {
            out.push(img.as_str());
        }
        out
    }
}

const READ_ONLY: ToolAnnotations = ToolAnnotations {
    read_only_hint: true,
    destructive_hint: false,
    idempotent_hint: true,
    open_world_hint: false,
};

const MUTATE_IDEMPOTENT: ToolAnnotations = ToolAnnotations {
    read_only_hint: false,
    destructive_hint: false,
    idempotent_hint: true,
    open_world_hint: false,
};

const MUTATE_NON_IDEMPOTENT: ToolAnnotations = ToolAnnotations {
    read_only_hint: false,
    destructive_hint: false,
    idempotent_hint: false,
    open_world_hint: false,
};

const DESTRUCTIVE: ToolAnnotations = ToolAnnotations {
    read_only_hint: false,
    destructive_hint: true,
    idempotent_hint: false,
    open_world_hint: false,
};

pub fn all_tools() -> Vec<ToolSpec> {
    let rgb_schema = json!({
        "type": "object",
        "properties": {
            "r": {"type": "number"},
            "g": {"type": "number"},
            "b": {"type": "number"}
        }
    });

    vec![
        ToolSpec {
            mcp_name: "project.list",
            llm_name: "project_list",
            description: "List all projects in the Solaris workspace.",
            input_schema: json!({"type": "object", "properties": {}}),
            annotations: READ_ONLY,
        },
        ToolSpec {
            mcp_name: "project.create",
            llm_name: "project_create",
            description: "Create a new Solaris 7-track project.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "aspect_preset": {"type": "string", "enum": ["vertical9x16", "landscape16x9", "square1x1"]},
                    "brand_playbook_id": {"type": "string"},
                    "prompt": {"type": "string"},
                    "template_id": {"type": "string"}
                },
                "required": ["name"]
            }),
            annotations: MUTATE_NON_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "project.run_qa",
            llm_name: "project_run_qa",
            description: "Run Stage-3 playback QA (hook/mid/climax frames, -14 LUFS loudness, social safe-zones, and GFX lint) with optional auto-fix.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "auto_fix": {"type": "boolean"}
                },
                "required": ["project_id"]
            }),
            annotations: MUTATE_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "project.get",
            llm_name: "project_get",
            description: "Fetch the full Project JSON (tracks, clips, media_pool, active_grade, revision).",
            input_schema: json!({
                "type": "object",
                "properties": { "project_id": {"type": "string"} },
                "required": ["project_id"]
            }),
            annotations: READ_ONLY,
        },
        ToolSpec {
            mcp_name: "project.undo",
            llm_name: "project_undo",
            description: "Undo the most recent operation on a project.",
            input_schema: json!({
                "type": "object",
                "properties": { "project_id": {"type": "string"} },
                "required": ["project_id"]
            }),
            annotations: MUTATE_NON_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "project.redo",
            llm_name: "project_redo",
            description: "Redo the most recently undone operation on a project.",
            input_schema: json!({
                "type": "object",
                "properties": { "project_id": {"type": "string"} },
                "required": ["project_id"]
            }),
            annotations: MUTATE_NON_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "project.lock_track",
            llm_name: "project_lock_track",
            description: "Acquire an exclusive agent lock on a track (t1, g1, v2, v1, fx, a1, a2).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "track_id": {"type": "string"}
                },
                "required": ["project_id", "track_id"]
            }),
            annotations: MUTATE_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "project.unlock_track",
            llm_name: "project_unlock_track",
            description: "Release an exclusive agent lock on a track.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "track_id": {"type": "string"}
                },
                "required": ["project_id", "track_id"]
            }),
            annotations: MUTATE_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "media.import",
            llm_name: "media_import",
            description: "Import local media file paths into a project's media pool.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "paths": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "path": {"type": "string"},
                                "role_hint": {"type": "string"}
                            },
                            "required": ["path"]
                        }
                    }
                },
                "required": ["project_id", "paths"]
            }),
            annotations: MUTATE_NON_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "media.get_manifest",
            llm_name: "media_get_manifest",
            description: "Return MediaIntelligenceManifest (duration, silences, beats, LUFS, log flag, transcript) for one or all media assets.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "media_id": {"type": "string"}
                },
                "required": ["project_id"]
            }),
            annotations: READ_ONLY,
        },
        ToolSpec {
            mcp_name: "media.transcribe",
            llm_name: "media_transcribe",
            description: "Run word-level transcription (local FFmpeg whisper or OpenRouter STT) on a media asset and store transcript in its manifest.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "media_id": {"type": "string"},
                    "language": {"type": "string"}
                },
                "required": ["project_id", "media_id"]
            }),
            annotations: MUTATE_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "media.extract_keyframes",
            llm_name: "media_extract_keyframes",
            description: "Extract PNG keyframes at requested timestamps from a media asset.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "media_id": {"type": "string"},
                    "timestamps_sec": {"type": "array", "items": {"type": "number"}}
                },
                "required": ["project_id", "media_id"]
            }),
            annotations: READ_ONLY,
        },
        ToolSpec {
            mcp_name: "timeline.place_clip",
            llm_name: "timeline_place_clip",
            description: "Place a media clip on a timeline track (v1, v2, a1, a2).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "track_id": {"type": "string"},
                    "clip_id": {"type": "string"},
                    "label": {"type": "string"},
                    "media_id": {"type": "string"},
                    "start_sec": {"type": "number"},
                    "duration_sec": {"type": "number"},
                    "source_in_sec": {"type": "number"},
                    "ken_burns": {"type": "boolean"},
                    "scale": {"type": "number"},
                    "gain_db": {"type": "number"},
                    "smart_reframe": {
                        "type": "object",
                        "properties": {
                            "x": {"type": "number"},
                            "y": {"type": "number"},
                            "width": {"type": "number"},
                            "height": {"type": "number"}
                        },
                        "required": ["x", "y", "width", "height"]
                    }
                },
                "required": ["project_id", "track_id", "clip_id", "media_id", "start_sec", "duration_sec"]
            }),
            annotations: MUTATE_NON_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "timeline.split_clip",
            llm_name: "timeline_split_clip",
            description: "Split an existing clip at a timeline timestamp.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "track_id": {"type": "string"},
                    "clip_id": {"type": "string"},
                    "split_at_timeline_sec": {"type": "number"},
                    "right_clip_id": {"type": "string"}
                },
                "required": ["project_id", "track_id", "clip_id", "split_at_timeline_sec", "right_clip_id"]
            }),
            annotations: MUTATE_NON_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "timeline.trim_clip",
            llm_name: "timeline_trim_clip",
            description: "Trim or slip a clip's start_sec, duration_sec, or source_in_sec.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "track_id": {"type": "string"},
                    "clip_id": {"type": "string"},
                    "new_start_sec": {"type": "number"},
                    "new_duration_sec": {"type": "number"},
                    "new_source_in_sec": {"type": "number"}
                },
                "required": ["project_id", "track_id", "clip_id"]
            }),
            annotations: MUTATE_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "timeline.delete_clip",
            llm_name: "timeline_delete_clip",
            description: "Delete a clip from a track.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "track_id": {"type": "string"},
                    "clip_id": {"type": "string"}
                },
                "required": ["project_id", "track_id", "clip_id"]
            }),
            annotations: DESTRUCTIVE,
        },
        ToolSpec {
            mcp_name: "timeline.set_transform",
            llm_name: "timeline_set_transform",
            description: "Set 2D transform (x, y, scale, rotation_deg, opacity) on a clip (e.g. 1.08 punch-in zoom).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "track_id": {"type": "string"},
                    "clip_id": {"type": "string"},
                    "x": {"type": "number"},
                    "y": {"type": "number"},
                    "scale": {"type": "number"},
                    "rotation_deg": {"type": "number"},
                    "opacity": {"type": "number"},
                    "smart_reframe": {
                        "type": "object",
                        "properties": {
                            "x": {"type": "number"},
                            "y": {"type": "number"},
                            "width": {"type": "number"},
                            "height": {"type": "number"}
                        },
                        "required": ["x", "y", "width", "height"]
                    }
                },
                "required": ["project_id", "track_id", "clip_id"]
            }),
            annotations: MUTATE_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "color.set_grade",
            llm_name: "color_set_grade",
            description: "Set project active_grade (or clip-scoped FX grade) with Lift/Gamma/Gain, temp, tint, contrast, saturation, exposure, brightness, and LUT.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "track_id": {"type": "string"},
                    "clip_id": {"type": "string"},
                    "lift": rgb_schema,
                    "gamma": rgb_schema,
                    "gain": rgb_schema,
                    "contrast": {"type": "number"},
                    "saturation": {"type": "number"},
                    "exposure": {"type": "number"},
                    "brightness": {"type": "number"},
                    "temp": {"type": "number"},
                    "tint": {"type": "number"},
                    "lut_path": {"type": ["string", "null"]},
                    "lut_intensity": {"type": "number"}
                },
                "required": ["project_id"]
            }),
            annotations: MUTATE_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "color.place_grade_clip",
            llm_name: "color_place_grade_clip",
            description: "Place a time-ranged FX color-grade adjustment clip on track fx for per-scene color matching.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "clip_id": {"type": "string"},
                    "label": {"type": "string"},
                    "start_sec": {"type": "number"},
                    "duration_sec": {"type": "number"},
                    "lift": rgb_schema,
                    "gamma": rgb_schema,
                    "gain": rgb_schema,
                    "contrast": {"type": "number"},
                    "saturation": {"type": "number"},
                    "exposure": {"type": "number"},
                    "brightness": {"type": "number"},
                    "temp": {"type": "number"},
                    "tint": {"type": "number"},
                    "lut_path": {"type": ["string", "null"]},
                    "lut_intensity": {"type": "number"}
                },
                "required": ["project_id", "clip_id", "start_sec", "duration_sec"]
            }),
            annotations: MUTATE_NON_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "color.get_scope_stats",
            llm_name: "color_get_scope_stats",
            description: "Inspect active color grade, brand LUT status, and log-footage flags across the project.",
            input_schema: json!({
                "type": "object",
                "properties": { "project_id": {"type": "string"} },
                "required": ["project_id"]
            }),
            annotations: READ_ONLY,
        },
        ToolSpec {
            mcp_name: "captions.generate_from_transcript",
            llm_name: "captions_generate_from_transcript",
            description: "Generate 2-4 word social safe-zone caption clips on T1 from stored media transcripts, aligned to placed V1 and A1 clips.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "media_id": {"type": "string"},
                    "preset": {"type": "string", "enum": ["neon_glow", "mono_caps", "karaoke", "minimal"]},
                    "words_per_burst": {"type": "integer"}
                },
                "required": ["project_id"]
            }),
            annotations: MUTATE_NON_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "captions.set_style",
            llm_name: "captions_set_style",
            description: "Update caption preset, font_family, or highlight_color across T1 caption clips.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "preset": {"type": "string", "enum": ["neon_glow", "mono_caps", "karaoke", "minimal"]},
                    "font_family": {"type": "string"},
                    "highlight_color": {"type": "string"}
                },
                "required": ["project_id", "preset"]
            }),
            annotations: MUTATE_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "captions.place_script",
            llm_name: "captions_place_script",
            description: "Place word-synced caption bursts on T1 for a script that IS spoken by narration audio already on the timeline but lacks a transcript. Never use on silent/no-audio videos: Solaris captions must reflect real narration.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "preset": {"type": "string", "enum": ["neon_glow", "mono_caps", "karaoke", "minimal"]},
                    "words_per_burst": {"type": "integer", "minimum": 1, "maximum": 6},
                    "y": {"type": "number", "minimum": -0.32, "maximum": 0.32},
                    "replace_existing": {"type": "boolean"},
                    "segments": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 64,
                        "items": {
                            "type": "object",
                            "properties": {
                                "text": {"type": "string"},
                                "start_sec": {"type": "number", "minimum": 0.0},
                                "duration_sec": {"type": "number", "minimum": 0.06}
                            },
                            "required": ["text", "start_sec", "duration_sec"]
                        }
                    }
                },
                "required": ["project_id", "segments"]
            }),
            annotations: MUTATE_NON_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "captions.update_clip",
            llm_name: "captions_update_clip",
            description: "Edit ONE existing T1 caption clip in place: replace its text (keeps transcript word timings when the word count is unchanged), preset, font_family, or highlight_color.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "clip_id": {"type": "string"},
                    "text": {"type": "string"},
                    "preset": {"type": "string", "enum": ["neon_glow", "mono_caps", "karaoke", "minimal"]},
                    "font_family": {"type": "string"},
                    "highlight_color": {"type": "string"}
                },
                "required": ["project_id", "clip_id"]
            }),
            annotations: MUTATE_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "assets.search",
            llm_name: "assets_search",
            description: "Search reusable HTML/CSS/GSAP motion-graphic SavedCodeAssets by query or category.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "category": {"type": "string"}
                }
            }),
            annotations: READ_ONLY,
        },
        ToolSpec {
            mcp_name: "assets.save",
            llm_name: "assets_save",
            description: "Lint and save a reusable HTML/GSAP GFX template into the SavedCodeAsset library.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "name": {"type": "string"},
                    "category": {"type": "string"},
                    "tags": {"type": "array", "items": {"type": "string"}},
                    "props_schema": {"type": "object"},
                    "default_props": {"type": "object"},
                    "html_source": {"type": "string"},
                    "default_duration_sec": {"type": "number"}
                },
                "required": ["id", "name", "html_source"]
            }),
            annotations: MUTATE_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "gfx.place",
            llm_name: "gfx_place",
            description: "Place a programmable HTML/GSAP GFX overlay clip on G1.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "clip_id": {"type": "string"},
                    "label": {"type": "string"},
                    "start_sec": {"type": "number"},
                    "duration_sec": {"type": "number"},
                    "asset_id": {
                        "type": "string",
                        "description": "Optional saved GFX template ID from assets_search; omit when providing inline_html."
                    },
                    "props": {"type": "object"},
                    "inline_html": {
                        "type": "string",
                        "description": "Full seekable HTML/CSS/SVG/GSAP snippet (<div class=\"clip\" data-start=\"0\" data-duration=\"...\">...</div>). Required unless referencing a saved template via asset_id."
                    }
                },
                "required": ["project_id", "clip_id", "label", "start_sec", "duration_sec"]
            }),
            annotations: MUTATE_NON_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "gfx.update_clip",
            llm_name: "gfx_update_clip",
            description: "Surgically edit ONE existing G1 GFX clip in place (same clip_id, single undo step): change label, start_sec, duration_sec, inline_html (auto-linted), asset_id, or deep-merge props. Use this instead of placing a duplicate clip when refining.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "clip_id": {"type": "string"},
                    "label": {"type": "string"},
                    "start_sec": {"type": "number", "minimum": 0.0},
                    "duration_sec": {"type": "number", "minimum": 0.05},
                    "asset_id": {"type": "string"},
                    "props": {"type": "object"},
                    "inline_html": {"type": "string"}
                },
                "required": ["project_id", "clip_id"]
            }),
            annotations: MUTATE_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "gfx.lint_html",
            llm_name: "gfx_lint_html",
            description: "Validate an HTML/CSS GFX snippet against Solaris seekable & sandbox rules.",
            input_schema: json!({
                "type": "object",
                "properties": { "html": {"type": "string"} },
                "required": ["html"]
            }),
            annotations: READ_ONLY,
        },
        ToolSpec {
            mcp_name: "audio.set_mix",
            llm_name: "audio_set_mix",
            description: "Set gain_db, ducking, and fade_in/fade_out on a timeline clip.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "track_id": {"type": "string"},
                    "clip_id": {"type": "string"},
                    "gain_db": {"type": "number"},
                    "ducking_enabled": {"type": "boolean"},
                    "duck_target_db": {"type": "number"},
                    "fade_in_sec": {"type": "number"},
                    "fade_out_sec": {"type": "number"}
                },
                "required": ["project_id", "track_id", "clip_id"]
            }),
            annotations: MUTATE_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "audio.normalize_lufs",
            llm_name: "audio_normalize_lufs",
            description: "Normalize voice/A-roll clip gain toward a target integrated LUFS (default -14.0).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "target_lufs": {"type": "number"}
                },
                "required": ["project_id"]
            }),
            annotations: MUTATE_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "audio.enable_ducking",
            llm_name: "audio_enable_ducking",
            description: "Enable voice-driven sidechain ducking on all music clips on A2.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "duck_target_db": {"type": "number"}
                },
                "required": ["project_id"]
            }),
            annotations: MUTATE_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "brand.list_playbooks",
            llm_name: "brand_list_playbooks",
            description: "List all Brand Playbooks.",
            input_schema: json!({"type": "object", "properties": {}}),
            annotations: READ_ONLY,
        },
        ToolSpec {
            mcp_name: "brand.get_playbook",
            llm_name: "brand_get_playbook",
            description: "Fetch a Brand Playbook (tokens, brand_md, pacing_rules, default_lut).",
            input_schema: json!({
                "type": "object",
                "properties": { "brand_id": {"type": "string"} },
                "required": ["brand_id"]
            }),
            annotations: READ_ONLY,
        },
        ToolSpec {
            mcp_name: "preview.seek",
            llm_name: "preview_seek",
            description: "Move the Studio UI Program Monitor playhead to a specific timestamp in seconds.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "time_sec": {"type": "number"}
                },
                "required": ["project_id", "time_sec"]
            }),
            annotations: MUTATE_IDEMPOTENT,
        },
        ToolSpec {
            mcp_name: "preview.capture_frame",
            llm_name: "preview_capture_frame",
            description: "Capture a PNG frame from the active timeline at time_sec for multimodal visual QA (supports both video/image tracks and programmatic G1/T1 overlay-only timelines).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "time_sec": {"type": "number"}
                },
                "required": ["project_id", "time_sec"]
            }),
            annotations: READ_ONLY,
        },
        ToolSpec {
            mcp_name: "render.export",
            llm_name: "render_export",
            description: "Start a dual-path FFmpeg + Chromium export job.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "format": {"type": "string", "enum": ["mp4", "mov_pro_res4444", "webm_alpha"]},
                    "quality": {"type": "string", "enum": ["draft", "standard", "high"]},
                    "target_lufs": {"type": "number"},
                    "output_name": {"type": "string"}
                },
                "required": ["project_id"]
            }),
            annotations: MUTATE_NON_IDEMPOTENT,
        },
    ]
}

pub fn tools_for_role(role: AgentRole) -> Vec<ToolDefinition> {
    let allowed: &[&str] = match role {
        AgentRole::Director => &[
            "project.get",
            "project.run_qa",
            "media.get_manifest",
            "media.transcribe",
            "media.extract_keyframes",
            "brand.get_playbook",
            "color.get_scope_stats",
            "assets.search",
            "preview.capture_frame",
        ],
        AgentRole::Razor => &[
            "project.get",
            "media.get_manifest",
            "media.transcribe",
            "media.extract_keyframes",
            "timeline.place_clip",
            "timeline.split_clip",
            "timeline.trim_clip",
            "timeline.delete_clip",
            "timeline.set_transform",
            "preview.capture_frame",
        ],
        AgentRole::BRoll => &[
            "project.get",
            "media.get_manifest",
            "media.extract_keyframes",
            "timeline.place_clip",
            "timeline.split_clip",
            "timeline.trim_clip",
            "timeline.delete_clip",
            "timeline.set_transform",
            "preview.capture_frame",
        ],
        AgentRole::Grade => &[
            "project.get",
            "media.get_manifest",
            "media.extract_keyframes",
            "brand.get_playbook",
            "color.get_scope_stats",
            "color.set_grade",
            "color.place_grade_clip",
            "preview.capture_frame",
        ],
        // Captions come strictly from transcribed narration; scripted captions are not
        // available to autonomous agents so silent videos never get phantom captions.
        AgentRole::Caps => &[
            "project.get",
            "media.get_manifest",
            "media.transcribe",
            "brand.get_playbook",
            "captions.generate_from_transcript",
            "captions.set_style",
            "captions.update_clip",
            "timeline.trim_clip",
            "timeline.delete_clip",
            "timeline.set_transform",
            "preview.capture_frame",
        ],
        AgentRole::Gfx => &[
            "project.get",
            "media.get_manifest",
            "media.extract_keyframes",
            "brand.get_playbook",
            "assets.search",
            "assets.save",
            "gfx.lint_html",
            "gfx.place",
            "gfx.update_clip",
            "timeline.trim_clip",
            "timeline.delete_clip",
            "timeline.set_transform",
            "preview.capture_frame",
        ],
        AgentRole::Mix => &[
            "project.get",
            "media.get_manifest",
            "timeline.place_clip",
            "timeline.trim_clip",
            "timeline.delete_clip",
            "audio.set_mix",
            "audio.normalize_lufs",
            "audio.enable_ducking",
        ],
        AgentRole::User | AgentRole::McpExternal => {
            return all_tools().into_iter().map(to_llm_def).collect();
        }
    };

    all_tools()
        .into_iter()
        .filter(|t| allowed.contains(&t.mcp_name))
        .map(to_llm_def)
        .collect()
}

pub fn tools_for_refinement(role: AgentRole) -> Vec<ToolDefinition> {
    tools_for_role(role)
        .into_iter()
        .filter(|def| {
            let name = normalize_tool_name(&def.function.name);
            name != "assets.save"
        })
        .collect()
}

pub fn is_mutating_tool(raw_name: &str) -> bool {
    let norm = normalize_tool_name(raw_name.trim());
    all_tools()
        .into_iter()
        .find(|t| t.mcp_name == norm)
        .is_some_and(|t| !t.annotations.read_only_hint)
}

fn to_llm_def(spec: ToolSpec) -> ToolDefinition {
    ToolDefinition {
        kind: "function".to_string(),
        function: ToolFunctionDef {
            name: spec.llm_name.to_string(),
            description: spec.description.to_string(),
            parameters: spec.input_schema,
        },
    }
}

pub fn normalize_tool_name(raw: &str) -> &str {
    for spec in [
        ("project_list", "project.list"),
        ("project_create", "project.create"),
        ("project_get", "project.get"),
        ("project_run_qa", "project.run_qa"),
        ("project_undo", "project.undo"),
        ("project_redo", "project.redo"),
        ("project_lock_track", "project.lock_track"),
        ("project_unlock_track", "project.unlock_track"),
        ("media_import", "media.import"),
        ("media_get_manifest", "media.get_manifest"),
        ("media_transcribe", "media.transcribe"),
        ("media_extract_keyframes", "media.extract_keyframes"),
        ("timeline_place_clip", "timeline.place_clip"),
        ("timeline_split_clip", "timeline.split_clip"),
        ("timeline_trim_clip", "timeline.trim_clip"),
        ("timeline_delete_clip", "timeline.delete_clip"),
        ("timeline_set_transform", "timeline.set_transform"),
        ("color_set_grade", "color.set_grade"),
        ("color_place_grade_clip", "color.place_grade_clip"),
        ("color_get_scope_stats", "color.get_scope_stats"),
        (
            "captions_generate_from_transcript",
            "captions.generate_from_transcript",
        ),
        ("captions_place_script", "captions.place_script"),
        ("captions_set_style", "captions.set_style"),
        ("captions_update_clip", "captions.update_clip"),
        ("assets_search", "assets.search"),
        ("assets_save", "assets.save"),
        ("gfx_place", "gfx.place"),
        ("gfx_update_clip", "gfx.update_clip"),
        ("gfx_lint_html", "gfx.lint_html"),
        ("audio_set_mix", "audio.set_mix"),
        ("audio_normalize_lufs", "audio.normalize_lufs"),
        ("audio_enable_ducking", "audio.enable_ducking"),
        ("brand_list_playbooks", "brand.list_playbooks"),
        ("brand_get_playbook", "brand.get_playbook"),
        ("preview_seek", "preview.seek"),
        ("preview_capture_frame", "preview.capture_frame"),
        ("render_export", "render.export"),
    ] {
        if raw == spec.0 {
            return spec.1;
        }
    }
    raw
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_tools_normalize_and_classify_mutability_consistently() {
        for spec in all_tools() {
            assert_eq!(
                normalize_tool_name(spec.llm_name),
                spec.mcp_name,
                "llm_name '{}' did not normalize to mcp_name '{}'",
                spec.llm_name,
                spec.mcp_name
            );
            assert_eq!(
                normalize_tool_name(spec.mcp_name),
                spec.mcp_name,
                "mcp_name '{}' did not stay idempotent under normalize_tool_name",
                spec.mcp_name
            );
            assert_eq!(
                is_mutating_tool(spec.llm_name),
                !spec.annotations.read_only_hint,
                "mutability mismatch for '{}'",
                spec.llm_name
            );
        }
    }
}
