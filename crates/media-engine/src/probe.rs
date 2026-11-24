use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use serde::Deserialize;

use crate::error::MediaError;
use crate::ffmpeg::MediaToolchain;

#[derive(Debug, Clone, PartialEq)]
pub struct ProbeSummary {
    pub duration_sec: f64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub codec: Option<String>,
    pub color_space: Option<String>,
    pub is_log_footage: bool,
    pub has_audio: bool,
    pub rotation_deg: i32,
}

#[derive(Debug, Deserialize)]
struct FfprobeOutput {
    #[serde(default)]
    streams: Vec<FfprobeStream>,
    format: Option<FfprobeFormat>,
}

#[derive(Debug, Deserialize, Default)]
struct FfprobeDisposition {
    attached_pic: Option<u8>,
}

#[derive(Debug, Deserialize, Default)]
struct FfprobeSideData {
    rotation: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct FfprobeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    r_frame_rate: Option<String>,
    avg_frame_rate: Option<String>,
    color_transfer: Option<String>,
    color_space: Option<String>,
    duration: Option<String>,
    disposition: Option<FfprobeDisposition>,
    #[serde(default)]
    tags: HashMap<String, String>,
    #[serde(default)]
    side_data_list: Vec<FfprobeSideData>,
}

#[derive(Debug, Deserialize)]
struct FfprobeFormat {
    duration: Option<String>,
}

pub fn probe_media(toolchain: &MediaToolchain, path: &Path) -> Result<ProbeSummary, MediaError> {
    let ffprobe = toolchain
        .ffprobe_path
        .as_ref()
        .ok_or_else(|| MediaError::ToolchainUnavailable("ffprobe not found".to_string()))?;

    let out = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .output()?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(MediaError::ProcessFailed(format!(
            "ffprobe failed: {stderr}"
        )));
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    parse_ffprobe_json(&stdout)
}

fn parse_valid_duration(raw: Option<&str>) -> Option<f64> {
    raw.and_then(|d| d.trim().parse::<f64>().ok())
        .filter(|&v| v.is_finite() && v >= 0.0)
}

fn extract_rotation_deg(stream: &FfprobeStream) -> i32 {
    if let Some(rot) = stream
        .side_data_list
        .iter()
        .filter_map(|sd| sd.rotation)
        .find(|r| r.is_finite())
    {
        return rot.round() as i32;
    }
    if let Some(rot) = stream
        .tags
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("rotate"))
        .and_then(|(_, v)| v.trim().parse::<f64>().ok())
        .filter(|r| r.is_finite())
    {
        return rot.round() as i32;
    }
    0
}

pub fn parse_ffprobe_json(raw: &str) -> Result<ProbeSummary, MediaError> {
    let parsed: FfprobeOutput =
        serde_json::from_str(raw).map_err(|e| MediaError::ParseError(e.to_string()))?;

    let video_stream = parsed.streams.iter().find(|s| {
        s.codec_type.as_deref() == Some("video")
            && s.disposition.as_ref().and_then(|d| d.attached_pic) != Some(1)
    });
    let audio_stream = parsed
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("audio"));

    // Reject explicitly non-finite duration values rather than silently accepting NaN/Inf
    for raw_dur in [
        parsed.format.as_ref().and_then(|f| f.duration.as_deref()),
        video_stream.and_then(|s| s.duration.as_deref()),
        audio_stream.and_then(|s| s.duration.as_deref()),
    ]
    .into_iter()
    .flatten()
    {
        if let Ok(v) = raw_dur.trim().parse::<f64>()
            && (!v.is_finite() || v < 0.0)
        {
            return Err(MediaError::ParseError(format!(
                "invalid non-finite or negative media duration '{raw_dur}'"
            )));
        }
    }

    let duration_sec =
        parse_valid_duration(parsed.format.as_ref().and_then(|f| f.duration.as_deref()))
            .or_else(|| parse_valid_duration(video_stream.and_then(|s| s.duration.as_deref())))
            .or_else(|| parse_valid_duration(audio_stream.and_then(|s| s.duration.as_deref())))
            .unwrap_or(0.0);

    let fps = video_stream
        .and_then(|s| s.avg_frame_rate.as_deref())
        .and_then(parse_rational_fps)
        .filter(|&v| v.is_finite() && v > 0.0 && v <= 240.0)
        .or_else(|| {
            video_stream
                .and_then(|s| s.r_frame_rate.as_deref())
                .and_then(parse_rational_fps)
                .filter(|&v| v.is_finite() && v > 0.0 && v <= 240.0)
        });

    let color_transfer = video_stream
        .and_then(|s| s.color_transfer.clone())
        .or_else(|| video_stream.and_then(|s| s.color_space.clone()));

    let is_log_footage = color_transfer.as_deref().is_some_and(|ct| {
        let lower = ct.to_lowercase();
        lower.contains("log")
            || lower.contains("slog")
            || lower.contains("arib-std-b67")
            || lower.contains("smpte2084")
    });

    let rotation_deg = video_stream.map(extract_rotation_deg).unwrap_or(0);
    let swap_dims = rotation_deg.rem_euclid(180) == 90;
    let raw_w = video_stream.and_then(|s| s.width);
    let raw_h = video_stream.and_then(|s| s.height);
    let (width, height) = if swap_dims {
        (raw_h, raw_w)
    } else {
        (raw_w, raw_h)
    };

    Ok(ProbeSummary {
        duration_sec,
        width,
        height,
        fps,
        codec: video_stream
            .and_then(|s| s.codec_name.clone())
            .or_else(|| audio_stream.and_then(|s| s.codec_name.clone())),
        color_space: color_transfer,
        is_log_footage,
        has_audio: audio_stream.is_some(),
        rotation_deg,
    })
}

fn parse_rational_fps(raw: &str) -> Option<f64> {
    if let Some((num_s, den_s)) = raw.split_once('/') {
        let num = num_s.parse::<f64>().ok()?;
        let den = den_s.parse::<f64>().ok()?;
        if den > 0.0 && num.is_finite() && den.is_finite() {
            return Some((num / den * 1000.0).round() / 1000.0);
        }
        return None;
    }
    raw.parse::<f64>().ok().filter(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ffprobe_video_and_log_metadata() {
        let sample = r#"{
            "streams": [
                {
                    "codec_type": "video",
                    "codec_name": "h264",
                    "width": 1080,
                    "height": 1920,
                    "r_frame_rate": "30000/1001",
                    "color_transfer": "slog3"
                },
                {
                    "codec_type": "audio",
                    "codec_name": "aac"
                }
            ],
            "format": { "duration": "34.433000" }
        }"#;

        let summary = parse_ffprobe_json(sample).unwrap();
        assert_eq!(summary.width, Some(1080));
        assert_eq!(summary.height, Some(1920));
        assert_eq!(summary.codec.as_deref(), Some("h264"));
        assert!(summary.is_log_footage);
        assert!(summary.has_audio);
        assert!((summary.duration_sec - 34.433).abs() < 1e-3);
        assert!((summary.fps.unwrap() - 29.97).abs() < 0.01);
    }

    #[test]
    fn ignores_attached_pic_album_art_and_prefers_avg_frame_rate() {
        let mp3_with_art = r#"{
            "streams": [
                { "codec_type": "audio", "codec_name": "mp3", "duration": "12.5" },
                {
                    "codec_type": "video",
                    "codec_name": "mjpeg",
                    "width": 500,
                    "height": 500,
                    "disposition": { "attached_pic": 1 }
                }
            ],
            "format": {}
        }"#;
        let mp3 = parse_ffprobe_json(mp3_with_art).unwrap();
        assert_eq!(mp3.width, None);
        assert_eq!(mp3.codec.as_deref(), Some("mp3"));
        assert!(mp3.has_audio);
        assert!((mp3.duration_sec - 12.5).abs() < 1e-3);

        let rotated = r#"{
            "streams": [
                {
                    "codec_type": "video",
                    "codec_name": "h264",
                    "width": 1920,
                    "height": 1080,
                    "avg_frame_rate": "30/1",
                    "side_data_list": [{ "rotation": -90 }]
                }
            ],
            "format": { "duration": "5.0" }
        }"#;
        let rot_sum = parse_ffprobe_json(rotated).unwrap();
        assert_eq!(rot_sum.width, Some(1080));
        assert_eq!(rot_sum.height, Some(1920));
        assert_eq!(rot_sum.rotation_deg, -90);

        let nan_dur = r#"{ "streams": [], "format": { "duration": "NaN" } }"#;
        assert!(parse_ffprobe_json(nan_dur).is_err());
    }
}
