use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use solaris_core_ir::{MediaIntelligenceManifest, MediaKind, SmartReframeBox, TimeSpan};

use crate::error::MediaError;
use crate::ffmpeg::MediaToolchain;
use crate::probe::probe_media;

const MAX_WAVEFORM_SAMPLES: usize = 7_200_000; // 30 minutes @ 4 kHz
const WAVEFORM_SAMPLE_RATE: f64 = 4000.0;

pub fn probe_and_analyze_fast(
    toolchain: &MediaToolchain,
    path: &Path,
    kind: MediaKind,
) -> MediaIntelligenceManifest {
    let mut manifest = MediaIntelligenceManifest::default();

    let probe = match probe_media(toolchain, path) {
        Ok(p) => p,
        Err(err) => {
            manifest.analysis_error = Some(err.to_string());
            return manifest;
        }
    };

    manifest.analyzed = true;
    manifest.duration_sec = probe.duration_sec;
    manifest.width = probe.width;
    manifest.height = probe.height;
    manifest.fps = probe.fps;
    manifest.codec = probe.codec;
    manifest.color_space = probe.color_space;
    manifest.is_log_footage = probe.is_log_footage;
    manifest.has_audio = probe.has_audio;
    manifest.default_reframe = estimate_default_reframe(probe.width, probe.height);

    if probe.has_audio || kind == MediaKind::Audio {
        if let Ok((peaks, beats)) = extract_waveform_and_beats(toolchain, path, 96) {
            manifest.waveform_peaks = peaks;
            manifest.beats = beats;
        }
        let dur_opt = (probe.duration_sec > 0.0).then_some(probe.duration_sec);
        if let Ok((silences, lufs)) =
            analyze_silences_and_loudness(toolchain, path, -35.0, 0.25, dur_opt)
        {
            manifest.silences = silences;
            manifest.integrated_lufs = lufs;
        }
    }

    if kind == MediaKind::Video
        && probe.duration_sec > 0.5
        && let Ok(cuts) = detect_scene_cuts(toolchain, path, 12.0)
    {
        manifest.scene_cuts = cuts;
    }

    manifest
}

/// Estimates a default 9:16 subject-centered smart reframe crop for landscape visual media.
pub fn estimate_default_reframe(
    width: Option<u32>,
    height: Option<u32>,
) -> Option<SmartReframeBox> {
    let (w, h) = (width? as f64, height? as f64);
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let aspect = w / h;
    let target_9x16 = 9.0 / 16.0;
    if aspect <= target_9x16 + 0.05 {
        return Some(SmartReframeBox {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        });
    }
    let crop_w = (target_9x16 / aspect).clamp(0.2, 1.0);
    let crop_x = ((1.0 - crop_w) * 0.5).clamp(0.0, 1.0 - crop_w);
    Some(SmartReframeBox {
        x: (crop_x * 10000.0).round() / 10000.0,
        y: 0.0,
        width: (crop_w * 10000.0).round() / 10000.0,
        height: 1.0,
    })
}

pub fn analyze_media_file(
    toolchain: &MediaToolchain,
    path: &Path,
    kind: MediaKind,
    proxy_output_path: Option<&Path>,
) -> MediaIntelligenceManifest {
    let mut manifest = probe_and_analyze_fast(toolchain, path, kind);
    if !manifest.analyzed {
        return manifest;
    }

    if kind == MediaKind::Video
        && let Some(proxy_out) = proxy_output_path
        && generate_proxy(toolchain, path, proxy_out, 720).is_ok()
    {
        manifest.proxy_path = Some(proxy_out.to_string_lossy().to_string());
    }

    manifest
}

pub fn detect_scene_cuts(
    toolchain: &MediaToolchain,
    path: &Path,
    threshold: f64,
) -> Result<Vec<f64>, MediaError> {
    let ffmpeg = toolchain
        .ffmpeg_path
        .as_ref()
        .ok_or_else(|| MediaError::ToolchainUnavailable("ffmpeg not found".to_string()))?;

    let clamped_t = threshold.clamp(3.0, 60.0);
    let filter = format!("scale=320:-2,scdet=t={clamped_t:.1},metadata=print:key=lavfi.scd.time");
    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-an", "-i"])
        .arg(path)
        .args(["-vf", &filter, "-f", "null", "-"])
        .output()?;

    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        return Err(MediaError::ProcessFailed(stderr.to_string()));
    }
    Ok(parse_scdet_stderr(&stderr))
}

pub fn parse_scdet_stderr(stderr: &str) -> Vec<f64> {
    let mut cuts: Vec<f64> = Vec::new();
    for line in stderr.lines() {
        if let Some(pos) = line.find("lavfi.scd.time=") {
            let val = line[pos + "lavfi.scd.time=".len()..].trim();
            if let Some(token) = val.split_whitespace().next()
                && let Ok(t) = token.parse::<f64>()
                && t.is_finite()
                && t > 0.05
                && cuts.last().is_none_or(|&prev| (t - prev).abs() >= 0.25)
            {
                cuts.push((t * 1000.0).round() / 1000.0);
            }
        }
    }
    cuts
}

pub fn analyze_silences_and_loudness(
    toolchain: &MediaToolchain,
    path: &Path,
    noise_db: f64,
    min_duration_sec: f64,
    total_duration_sec: Option<f64>,
) -> Result<(Vec<TimeSpan>, Option<f64>), MediaError> {
    let ffmpeg = toolchain
        .ffmpeg_path
        .as_ref()
        .ok_or_else(|| MediaError::ToolchainUnavailable("ffmpeg not found".to_string()))?;

    let filter =
        format!("silencedetect=n={noise_db}dB:d={min_duration_sec},ebur128=framelog=verbose");
    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-vn", "-i"])
        .arg(path)
        .args(["-af", &filter, "-f", "null", "-"])
        .output()?;

    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        return Err(MediaError::ProcessFailed(stderr.to_string()));
    }

    let silences = parse_silencedetect_stderr_with_duration(&stderr, total_duration_sec);
    let lufs = parse_ebur128_integrated_lufs(&stderr);
    Ok((silences, lufs))
}

pub fn detect_silences(
    toolchain: &MediaToolchain,
    path: &Path,
    noise_db: f64,
    min_duration_sec: f64,
) -> Result<Vec<TimeSpan>, MediaError> {
    let ffmpeg = toolchain
        .ffmpeg_path
        .as_ref()
        .ok_or_else(|| MediaError::ToolchainUnavailable("ffmpeg not found".to_string()))?;

    let probed_duration = probe_media(toolchain, path)
        .ok()
        .map(|p| p.duration_sec)
        .filter(|&d| d.is_finite() && d > 0.0);

    let filter = format!("silencedetect=n={noise_db}dB:d={min_duration_sec}");
    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-vn", "-i"])
        .arg(path)
        .args(["-af", &filter, "-f", "null", "-"])
        .output()?;

    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        return Err(MediaError::ProcessFailed(stderr.to_string()));
    }
    Ok(parse_silencedetect_stderr_with_duration(
        &stderr,
        probed_duration,
    ))
}

pub fn parse_silencedetect_stderr(stderr: &str) -> Vec<TimeSpan> {
    parse_silencedetect_stderr_with_duration(stderr, None)
}

pub fn parse_silencedetect_stderr_with_duration(
    stderr: &str,
    total_duration_sec: Option<f64>,
) -> Vec<TimeSpan> {
    let mut spans = Vec::new();
    let mut current_start: Option<f64> = None;

    for line in stderr.lines() {
        if let Some(pos) = line.find("silence_start:") {
            let val = line[pos + "silence_start:".len()..].trim();
            if let Some(token) = val.split_whitespace().next()
                && let Ok(t) = token.parse::<f64>()
                && t.is_finite()
            {
                current_start = Some(t.max(0.0));
            }
        } else if let Some(pos) = line.find("silence_end:") {
            let val = line[pos + "silence_end:".len()..].trim();
            if let Some(token) = val.split_whitespace().next()
                && let Ok(end) = token.parse::<f64>()
                && end.is_finite()
                && let Some(start) = current_start.take()
            {
                let clamped_end = end.max(0.0);
                if clamped_end > start {
                    spans.push(TimeSpan {
                        start_sec: start,
                        end_sec: clamped_end,
                    });
                }
            }
        }
    }

    if let (Some(start), Some(total)) = (current_start, total_duration_sec)
        && total.is_finite()
        && total > start
    {
        spans.push(TimeSpan {
            start_sec: start,
            end_sec: total,
        });
    }

    spans
}

pub fn measure_loudness(
    toolchain: &MediaToolchain,
    path: &Path,
) -> Result<Option<f64>, MediaError> {
    let ffmpeg = toolchain
        .ffmpeg_path
        .as_ref()
        .ok_or_else(|| MediaError::ToolchainUnavailable("ffmpeg not found".to_string()))?;

    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-vn", "-i"])
        .arg(path)
        .args(["-af", "ebur128=framelog=verbose", "-f", "null", "-"])
        .output()?;

    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        return Err(MediaError::ProcessFailed(stderr.to_string()));
    }
    Ok(parse_ebur128_integrated_lufs(&stderr))
}

pub fn parse_ebur128_integrated_lufs(stderr: &str) -> Option<f64> {
    let mut last_i: Option<f64> = None;
    for line in stderr.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("I:")
            && let Some(token) = rest.split_whitespace().next()
            && let Ok(val) = token.parse::<f64>()
            && val.is_finite()
        {
            last_i = Some(val);
        }
    }
    last_i
}

pub fn extract_waveform_peaks(
    toolchain: &MediaToolchain,
    path: &Path,
    bucket_count: usize,
) -> Result<Vec<f32>, MediaError> {
    let (peaks, _beats) = extract_waveform_and_beats(toolchain, path, bucket_count)?;
    Ok(peaks)
}

pub fn extract_waveform_and_beats(
    toolchain: &MediaToolchain,
    path: &Path,
    bucket_count: usize,
) -> Result<(Vec<f32>, Vec<f64>), MediaError> {
    let ffmpeg = toolchain
        .ffmpeg_path
        .as_ref()
        .ok_or_else(|| MediaError::ToolchainUnavailable("ffmpeg not found".to_string()))?;

    let mut child = Command::new(ffmpeg)
        .args(["-hide_banner", "-v", "error", "-vn", "-i"])
        .arg(path)
        .args([
            "-t", "1800", "-ac", "1", "-ar", "4000", "-f", "f32le", "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Drain stderr concurrently on a background thread so FFmpeg can never deadlock on a full pipe.
    let stderr_handle = child.stderr.take().map(|mut err_pipe| {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = err_pipe.read_to_string(&mut buf);
            buf
        })
    });

    let mut samples: Vec<f32> = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        let mut buf = [0u8; 65_536];
        let mut carry = [0u8; 4];
        let mut carry_len = 0usize;

        loop {
            let n = stdout.read(&mut buf)?;
            if n == 0 {
                break;
            }
            let mut slice = &buf[..n];
            if carry_len > 0 {
                let need = 4 - carry_len;
                if slice.len() >= need {
                    carry[carry_len..4].copy_from_slice(&slice[..need]);
                    if samples.len() < MAX_WAVEFORM_SAMPLES {
                        samples.push(f32::from_le_bytes(carry));
                    }
                    slice = &slice[need..];
                    carry_len = 0;
                } else {
                    carry[carry_len..carry_len + slice.len()].copy_from_slice(slice);
                    carry_len += slice.len();
                    continue;
                }
            }
            let (chunks, rem) = slice.as_chunks::<4>();
            for b in chunks {
                if samples.len() >= MAX_WAVEFORM_SAMPLES {
                    break;
                }
                samples.push(f32::from_le_bytes(*b));
            }
            if !rem.is_empty() {
                carry[..rem.len()].copy_from_slice(rem);
                carry_len = rem.len();
            }
        }
    }

    let status = child.wait()?;
    let stderr_text = stderr_handle
        .and_then(|h| h.join().ok())
        .unwrap_or_default();
    if !status.success() {
        return Err(MediaError::ProcessFailed(stderr_text));
    }

    let peaks = compute_peaks_from_f32_samples(&samples, bucket_count);
    let beats = detect_beats_from_f32_samples(&samples, WAVEFORM_SAMPLE_RATE);
    Ok((peaks, beats))
}

pub fn compute_peaks_from_f32_samples(samples: &[f32], bucket_count: usize) -> Vec<f32> {
    if samples.is_empty() || bucket_count == 0 {
        return Vec::new();
    }
    let n = samples.len();
    let buckets = bucket_count.min(n);
    let mut peaks = Vec::with_capacity(buckets);
    for i in 0..buckets {
        let start = (i * n) / buckets;
        let end = (((i + 1) * n) / buckets).max(start + 1).min(n);
        let peak = samples[start..end]
            .iter()
            .filter(|s| s.is_finite())
            .map(|s| s.abs().clamp(0.0, 1.0))
            .fold(0.0_f32, f32::max);
        peaks.push(peak);
    }
    peaks
}

/// Lightweight onset/energy peak beat detector running directly on decoded mono PCM samples.
pub fn detect_beats_from_f32_samples(samples: &[f32], sample_rate: f64) -> Vec<f64> {
    if samples.is_empty() || !sample_rate.is_finite() || sample_rate <= 0.0 {
        return Vec::new();
    }
    let hop = ((sample_rate * 0.025).round() as usize).max(1); // 25ms hop
    let win = ((sample_rate * 0.050).round() as usize).max(hop); // 50ms window
    if samples.len() < win {
        return Vec::new();
    }

    let num_frames = (samples.len() - win) / hop + 1;
    let mut energies = Vec::with_capacity(num_frames);
    for idx in 0..num_frames {
        let start = idx * hop;
        let end = (start + win).min(samples.len());
        let sum_sq: f64 = samples[start..end]
            .iter()
            .filter(|s| s.is_finite())
            .map(|&s| {
                let v = f64::from(s);
                v * v
            })
            .sum();
        energies.push((sum_sq / (end - start).max(1) as f64).sqrt());
    }

    // Half-wave rectified energy flux
    let mut flux = vec![0.0_f64; energies.len()];
    for i in 1..energies.len() {
        flux[i] = (energies[i] - energies[i - 1]).max(0.0);
    }
    if !energies.is_empty() {
        flux[0] = energies[0];
    }

    let mut beats = Vec::new();
    let local_radius = 12usize; // ~300ms each side
    let min_gap_sec = 0.22; // up to ~272 BPM
    let mut last_beat = -min_gap_sec;

    for i in 0..flux.len() {
        let lo = i.saturating_sub(local_radius);
        let hi = (i + local_radius + 1).min(flux.len());
        let local_mean = flux[lo..hi].iter().sum::<f64>() / (hi - lo).max(1) as f64;
        let is_local_max =
            (i == 0 || flux[i] >= flux[i - 1]) && (i + 1 == flux.len() || flux[i] > flux[i + 1]);
        if is_local_max && flux[i] > 0.035 && flux[i] > local_mean * 1.65 {
            let t = (i * hop) as f64 / sample_rate;
            if t - last_beat >= min_gap_sec {
                beats.push((t * 1000.0).round() / 1000.0);
                last_beat = t;
            }
        }
    }

    beats
}

pub fn generate_proxy(
    toolchain: &MediaToolchain,
    input: &Path,
    output: &Path,
    max_height: u32,
) -> Result<(), MediaError> {
    let caps = toolchain.detect_capabilities();
    generate_proxy_with_encoders(toolchain, input, output, max_height, &caps.hw_encoders)
}

pub fn generate_proxy_with_encoders(
    toolchain: &MediaToolchain,
    input: &Path,
    output: &Path,
    max_height: u32,
    hw_encoders: &[String],
) -> Result<(), MediaError> {
    let ffmpeg = toolchain
        .ffmpeg_path
        .as_ref()
        .ok_or_else(|| MediaError::ToolchainUnavailable("ffmpeg not found".to_string()))?;

    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }

    let even_max_h = (max_height.max(2)) & !1;
    let scale_filter = format!("scale=-2:'trunc(min({even_max_h},ih)/2)*2'");
    let encoder = if hw_encoders.iter().any(|e| e == "h264_videotoolbox") {
        "h264_videotoolbox"
    } else if hw_encoders.iter().any(|e| e == "h264_nvenc") {
        "h264_nvenc"
    } else {
        "libx264"
    };

    let run_with_encoder = |enc: &str| -> Result<(), MediaError> {
        let mut cmd = Command::new(ffmpeg);
        cmd.args(["-y", "-hide_banner", "-v", "error", "-i"])
            .arg(input)
            .args(["-vf", &scale_filter, "-pix_fmt", "yuv420p", "-c:v", enc]);
        match enc {
            "libx264" => {
                cmd.args(["-preset", "veryfast", "-crf", "26"]);
            }
            "h264_nvenc" => {
                cmd.args(["-preset", "p4", "-b:v", "2500k"]);
            }
            _ => {
                cmd.args(["-b:v", "2500k"]);
            }
        }
        cmd.args(["-c:a", "aac", "-b:a", "128k", "-movflags", "+faststart"])
            .arg(output);

        let out = cmd.output()?;
        if !out.status.success() {
            return Err(MediaError::ProcessFailed(
                String::from_utf8_lossy(&out.stderr).to_string(),
            ));
        }
        Ok(())
    };

    match run_with_encoder(encoder) {
        Ok(()) => Ok(()),
        Err(hw_err) if encoder != "libx264" => {
            tracing::warn!(
                encoder,
                "hardware proxy encode failed ({hw_err}); falling back to libx264"
            );
            run_with_encoder("libx264")
        }
        Err(e) => Err(e),
    }
}

/// Generate a compact 480p low-FPS proxy MP4 suitable for native multimodal LLM video/audio inspection.
pub fn generate_llm_video_proxy(
    toolchain: &MediaToolchain,
    input: &Path,
    output: &Path,
    max_duration_sec: f64,
) -> Result<PathBuf, MediaError> {
    let ffmpeg = toolchain
        .ffmpeg_path
        .as_ref()
        .ok_or_else(|| MediaError::ToolchainUnavailable("ffmpeg not found".to_string()))?;

    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }

    let capped_dur = if max_duration_sec.is_finite() && max_duration_sec > 0.5 {
        max_duration_sec.min(60.0)
    } else {
        30.0
    };

    let mut child = Command::new(ffmpeg)
        .args(["-y", "-hide_banner", "-v", "error", "-i"])
        .arg(input)
        .args([
            "-t",
            &format!("{capped_dur:.2}"),
            "-vf",
            "scale=-2:'trunc(min(480,ih)/2)*2',fps=4",
            "-pix_fmt",
            "yuv420p",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "32",
            "-maxrate",
            "350k",
            "-bufsize",
            "700k",
            "-c:a",
            "aac",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-b:a",
            "32k",
            "-movflags",
            "+faststart",
        ])
        .arg(output)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;

    let stderr_handle = child.stderr.take().map(|mut err_pipe| {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = err_pipe.read_to_string(&mut buf);
            buf
        })
    });

    let status = child.wait()?;
    let stderr = stderr_handle
        .and_then(|h| h.join().ok())
        .unwrap_or_default();
    if !status.success() {
        return Err(MediaError::ProcessFailed(stderr));
    }

    let valid = output.metadata().is_ok_and(|m| m.is_file() && m.len() > 0);
    if !valid {
        return Err(MediaError::ProcessFailed(format!(
            "ffmpeg produced empty LLM video proxy at '{}'",
            output.display()
        )));
    }
    Ok(output.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_silencedetect_output_including_trailing_eof_and_clamps_negative() {
        let log = r#"
[silencedetect @ 0x123] silence_start: -0.020
[silencedetect @ 0x123] silence_end: 0.450 | silence_duration: 0.470
[silencedetect @ 0x123] silence_start: 1.245
[silencedetect @ 0x123] silence_end: 1.890 | silence_duration: 0.645
[silencedetect @ 0x123] silence_start: 5.100
"#;
        let spans = parse_silencedetect_stderr_with_duration(log, Some(6.0));
        assert_eq!(spans.len(), 3);
        assert!((spans[0].start_sec - 0.0).abs() < 1e-6);
        assert!((spans[0].end_sec - 0.450).abs() < 1e-3);
        assert!((spans[1].start_sec - 1.245).abs() < 1e-3);
        assert!((spans[2].start_sec - 5.100).abs() < 1e-3);
        assert!((spans[2].end_sec - 6.000).abs() < 1e-3);
    }

    #[test]
    fn parses_ebur128_integrated_lufs_and_ignores_non_finite() {
        let log = r#"
  Summary:
    Integrated loudness:
      I:         -14.2 LUFS
      Threshold: -24.5 LUFS
"#;
        let lufs = parse_ebur128_integrated_lufs(log).unwrap();
        assert!((lufs - (-14.2)).abs() < 1e-3);

        let silent_log = "  I:         -inf LUFS\n";
        assert_eq!(parse_ebur128_integrated_lufs(silent_log), None);
    }

    #[test]
    fn detects_beats_from_periodic_pulses_and_parses_scdet() {
        // Synthesize 3 seconds of 4kHz PCM with percussive impulses every 0.5s (120 BPM)
        let sr = 4000usize;
        let mut pcm = vec![0.0_f32; sr * 3];
        for beat_idx in 0..6 {
            let start = beat_idx * (sr / 2) + 200;
            for offset in 0..80 {
                if start + offset < pcm.len() {
                    pcm[start + offset] = 0.85 * (1.0 - offset as f32 / 80.0);
                }
            }
        }
        let beats = detect_beats_from_f32_samples(&pcm, sr as f64);
        assert!(beats.len() >= 4, "expected at least 4 beats, got {beats:?}");

        let scdet_log =
            "frame:12 pts:36000 pts_time:1.2\nlavfi.scd.time=1.200000\nlavfi.scd.time=4.750000\n";
        let cuts = parse_scdet_stderr(scdet_log);
        assert_eq!(cuts, vec![1.2, 4.75]);

        let reframe = estimate_default_reframe(Some(1920), Some(1080)).unwrap();
        assert!(reframe.width > 0.31 && reframe.width < 0.32);
        assert!(reframe.x > 0.34 && reframe.x < 0.35);
    }

    #[test]
    fn computes_waveform_peaks_proportionally_without_tail_truncation() {
        let samples = vec![0.1, -0.8, 0.2, 0.4, -0.95, 0.3];
        let peaks = compute_peaks_from_f32_samples(&samples, 3);
        assert_eq!(peaks.len(), 3);
        assert!((peaks[0] - 0.8).abs() < 1e-6);
        assert!((peaks[1] - 0.4).abs() < 1e-6);
        assert!((peaks[2] - 0.95).abs() < 1e-6);

        let mut long_samples = vec![0.1_f32; 190];
        long_samples[189] = 0.99;
        let long_peaks = compute_peaks_from_f32_samples(&long_samples, 96);
        assert_eq!(long_peaks.len(), 96);
        assert!((long_peaks[95] - 0.99).abs() < 1e-6);
    }
}
