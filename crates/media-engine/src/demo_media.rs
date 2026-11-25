use std::f64::consts::PI;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use uuid::Uuid;

use crate::error::MediaError;
use crate::ffmpeg::MediaToolchain;

const DEMO_PACK_VERSION: &str = "v1";
const PLATE_WIDTH: u32 = 540;
const PLATE_HEIGHT: u32 = 960;
const SAMPLE_RATE: u32 = 44_100;

#[derive(Debug, Clone)]
pub struct DemoMediaFile {
    pub path: PathBuf,
    pub is_video: bool,
    pub duration_sec: f64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub has_audio: bool,
    pub waveform_peaks: Vec<f32>,
    pub beats: Vec<f64>,
    pub integrated_lufs: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct DemoMediaPack {
    pub a_roll: DemoMediaFile,
    pub b_roll_workflow: DemoMediaFile,
    pub b_roll_grade: DemoMediaFile,
    pub voice_stem: DemoMediaFile,
    pub music_beat: DemoMediaFile,
}

/// Ensures a cached, self-contained, playable Demo Studio media pack exists inside
/// `<workspace_dir>/demo_media/v1/`. Generates real stereo WAV stems purely in Rust
/// and real studio visual plates (encoded to smooth H.264 MP4s when FFmpeg is available,
/// or crisp PNG plates for Ken Burns WebGL playback when FFmpeg is absent).
pub fn ensure_demo_media_pack(
    workspace_dir: &Path,
    toolchain: &MediaToolchain,
) -> Result<DemoMediaPack, MediaError> {
    let dir = workspace_dir.join("demo_media").join(DEMO_PACK_VERSION);
    fs::create_dir_all(&dir)?;

    let (music_path, music_peaks, music_beats) = ensure_music_wav(&dir)?;
    let (voice_path, voice_peaks) = ensure_voice_cadence_wav(&dir)?;

    let a_roll = ensure_visual_clip(
        &dir,
        "founder_studio_slog3",
        16.0,
        VisualScene::FounderStudio,
        toolchain,
        Some(&voice_path),
        voice_peaks.clone(),
        Some(-14.2),
    )?;

    let b_roll_workflow = ensure_visual_clip(
        &dir,
        "workflow_broll_solar",
        5.0,
        VisualScene::WorkflowOrbital,
        toolchain,
        None,
        Vec::new(),
        None,
    )?;

    let b_roll_grade = ensure_visual_clip(
        &dir,
        "studio_grade_broll",
        4.0,
        VisualScene::SolarHorizonGrade,
        toolchain,
        None,
        Vec::new(),
        None,
    )?;

    Ok(DemoMediaPack {
        a_roll,
        b_roll_workflow,
        b_roll_grade,
        voice_stem: DemoMediaFile {
            path: voice_path,
            is_video: false,
            duration_sec: 16.0,
            width: None,
            height: None,
            fps: None,
            has_audio: true,
            waveform_peaks: voice_peaks,
            beats: Vec::new(),
            integrated_lufs: Some(-14.0),
        },
        music_beat: DemoMediaFile {
            path: music_path,
            is_video: false,
            duration_sec: 16.0,
            width: None,
            height: None,
            fps: None,
            has_audio: true,
            waveform_peaks: music_peaks,
            beats: music_beats,
            integrated_lufs: Some(-14.2),
        },
    })
}

#[derive(Clone, Copy)]
enum VisualScene {
    FounderStudio,
    WorkflowOrbital,
    SolarHorizonGrade,
}

#[allow(clippy::too_many_arguments)]
fn ensure_visual_clip(
    dir: &Path,
    stem: &str,
    duration_sec: f64,
    scene: VisualScene,
    toolchain: &MediaToolchain,
    mux_audio_wav: Option<&Path>,
    waveform_peaks: Vec<f32>,
    integrated_lufs: Option<f64>,
) -> Result<DemoMediaFile, MediaError> {
    let mp4_path = dir.join(format!("{stem}.mp4"));
    let png_path = dir.join(format!("{stem}.png"));

    if is_non_empty_file(&mp4_path) {
        return Ok(DemoMediaFile {
            path: mp4_path,
            is_video: true,
            duration_sec,
            width: Some(PLATE_WIDTH),
            height: Some(PLATE_HEIGHT),
            fps: Some(30.0),
            has_audio: mux_audio_wav.is_some(),
            waveform_peaks,
            beats: Vec::new(),
            integrated_lufs,
        });
    }

    if !is_non_empty_file(&png_path) {
        let rgba = render_visual_plate(PLATE_WIDTH, PLATE_HEIGHT, scene);
        write_rgba_png(&png_path, PLATE_WIDTH, PLATE_HEIGHT, &rgba)?;
    }

    if let Some(ffmpeg) = &toolchain.ffmpeg_path {
        match encode_animated_plate_mp4(
            ffmpeg,
            &png_path,
            &mp4_path,
            duration_sec,
            scene,
            mux_audio_wav,
        ) {
            Ok(()) if is_non_empty_file(&mp4_path) => {
                return Ok(DemoMediaFile {
                    path: mp4_path,
                    is_video: true,
                    duration_sec,
                    width: Some(PLATE_WIDTH),
                    height: Some(PLATE_HEIGHT),
                    fps: Some(30.0),
                    has_audio: mux_audio_wav.is_some(),
                    waveform_peaks,
                    beats: Vec::new(),
                    integrated_lufs,
                });
            }
            Ok(()) => {
                tracing::warn!(
                    stem,
                    path = %mp4_path.display(),
                    "demo mp4 encode produced empty file; falling back to PNG plate"
                );
            }
            Err(err) => {
                tracing::warn!(
                    stem,
                    error = %err,
                    "demo mp4 encode failed; falling back to PNG plate"
                );
            }
        }
    }

    Ok(DemoMediaFile {
        path: png_path,
        is_video: false,
        duration_sec,
        width: Some(PLATE_WIDTH),
        height: Some(PLATE_HEIGHT),
        fps: None,
        has_audio: false,
        waveform_peaks: Vec::new(),
        beats: Vec::new(),
        integrated_lufs: None,
    })
}

fn is_non_empty_file(path: &Path) -> bool {
    path.metadata().is_ok_and(|m| m.is_file() && m.len() > 256)
}

struct TempFileGuard(Option<PathBuf>);

impl TempFileGuard {
    fn new(path: PathBuf) -> Self {
        Self(Some(path))
    }

    fn path(&self) -> &Path {
        self.0
            .as_deref()
            .expect("temp file guard accessed after commit")
    }

    fn commit(mut self, dest: &Path) -> Result<(), MediaError> {
        if let Some(tmp) = self.0.take()
            && let Err(err) = fs::rename(&tmp, dest)
        {
            let _ = fs::remove_file(&tmp);
            return Err(err.into());
        }
        Ok(())
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if let Some(tmp) = self.0.take() {
            let _ = fs::remove_file(&tmp);
        }
    }
}

fn unique_temp_path(dest: &Path, ext: &str) -> PathBuf {
    let stem = dest
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("demo_tmp");
    dest.with_file_name(format!(".{stem}.{}.tmp.{ext}", Uuid::new_v4()))
}

fn build_plate_vf(scene: VisualScene, duration_sec: f64) -> String {
    let dur_str = format!("{duration_sec:.2}");
    // Smooth cinematic camera drift & subtle lighting breath via animated crop + eq
    match scene {
        VisualScene::FounderStudio => {
            "scale=608:1080:flags=bicubic,crop=540:960:x='34+22*sin(t*0.42)':y='60+26*cos(t*0.31)',eq=brightness='0.015*sin(t*0.8)':saturation='1.0+0.04*sin(t*0.5)':eval=frame,format=yuv420p"
                .to_string()
        }
        VisualScene::WorkflowOrbital => format!(
            "scale=620:1102:flags=bicubic,crop=540:960:x='40+30*sin(t*0.75)':y='71-45*(t/{dur_str})',hue=h='4*sin(t*0.9)',format=yuv420p"
        ),
        VisualScene::SolarHorizonGrade => format!(
            "scale=620:1102:flags=bicubic,crop=540:960:x='40-26*cos(t*0.65)':y='96-60*(t/{dur_str})',eq=contrast='1.0+0.05*sin(t*0.9)':eval=frame,format=yuv420p"
        ),
    }
}

fn encode_animated_plate_mp4(
    ffmpeg: &Path,
    png_path: &Path,
    mp4_path: &Path,
    duration_sec: f64,
    scene: VisualScene,
    mux_audio_wav: Option<&Path>,
) -> Result<(), MediaError> {
    let tmp_guard = TempFileGuard::new(unique_temp_path(mp4_path, "mp4"));
    let dur_str = format!("{duration_sec:.2}");
    let vf = build_plate_vf(scene, duration_sec);

    let mut cmd = Command::new(ffmpeg);
    cmd.args([
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-loop",
        "1",
        "-framerate",
        "30",
        "-i",
    ])
    .arg(png_path);

    if let Some(wav) = mux_audio_wav {
        cmd.arg("-i").arg(wav);
    }

    cmd.args(["-t", &dur_str, "-vf", &vf, "-r", "30"]);

    if mux_audio_wav.is_some() {
        cmd.args([
            "-map", "0:v:0", "-map", "1:a:0", "-c:a", "aac", "-b:a", "128k",
        ]);
    } else {
        cmd.arg("-an");
    }

    cmd.args([
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-crf",
        "23",
        "-pix_fmt",
        "yuv420p",
        "-movflags",
        "+faststart",
    ])
    .arg(tmp_guard.path())
    .stdout(Stdio::null())
    .stderr(Stdio::piped());

    let output = cmd
        .output()
        .map_err(|e| MediaError::ProcessFailed(format!("demo mp4 ffmpeg spawn failed: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(MediaError::ProcessFailed(format!(
            "demo mp4 ffmpeg exited {:?}: {stderr}",
            output.status.code()
        )));
    }

    tmp_guard.commit(mp4_path)?;
    Ok(())
}

/// Renders rich, art-directed 9:16 editorial visual plates with wide shadow-to-highlight
/// latitude and warm skin-tone / solar hues so WebGL2 color grading & broadcast scopes shine.
fn render_visual_plate(w: u32, h: u32, scene: VisualScene) -> Vec<u8> {
    let mut pixels = vec![0u8; (w as usize) * (h as usize) * 4];
    let wf = w as f64;
    let hf = h as f64;

    for y in 0..h {
        let ny = y as f64 / hf;
        for x in 0..w {
            let nx = x as f64 / wf;

            let (mut r, mut g, mut b) = match scene {
                VisualScene::FounderStudio => sample_founder_studio(nx, ny),
                VisualScene::WorkflowOrbital => sample_workflow_orbital(nx, ny),
                VisualScene::SolarHorizonGrade => sample_solar_horizon_grade(nx, ny),
            };

            // Subtle vignette for studio focus
            let dx = (nx - 0.5) * 1.15;
            let dy = (ny - 0.48) * 0.95;
            let vig = (1.0 - (dx * dx + dy * dy) * 0.58).clamp(0.45, 1.0);
            r *= vig;
            g *= vig;
            b *= vig;

            // Subtle deterministic photographic film grain
            let hash = ((x.wrapping_mul(374_761_393)) ^ (y.wrapping_mul(668_265_263)))
                .wrapping_mul(1_274_126_177);
            let grain = (((hash >> 24) as f64 / 255.0) - 0.5) * 0.022;

            let idx = ((y * w + x) * 4) as usize;
            pixels[idx] = to_u8(r + grain);
            pixels[idx + 1] = to_u8(g + grain);
            pixels[idx + 2] = to_u8(b + grain);
            pixels[idx + 3] = 255;
        }
    }

    pixels
}

fn sample_founder_studio(nx: f64, ny: f64) -> (f64, f64, f64) {
    // Deep studio wall backdrop with warm amber practical light + subtle cool shadow fill
    let mut r = 0.075 + 0.055 * (1.0 - ny);
    let mut g = 0.052 + 0.032 * (1.0 - ny);
    let mut b = 0.068 + 0.048 * ny;

    // Warm key-light bloom from upper-right (gives authentic warm skin-tone vector)
    let key_d = hypot2((nx - 0.72) * 1.05, (ny - 0.28) * 1.35);
    let key = (-key_d * key_d * 3.6).exp();
    r += key * 0.58;
    g += key * 0.30;
    b += key * 0.13;

    // Cool teal studio rim fill on lower-left for rich Rec.709 color separation
    let fill_d = hypot2((nx - 0.14) * 1.2, (ny - 0.62) * 1.1);
    let fill = (-fill_d * fill_d * 4.2).exp();
    r += fill * 0.07;
    g += fill * 0.21;
    b += fill * 0.31;

    // Warm tungsten bokeh orbs in background
    for &(bx, by, rad, br, bg, bb) in &[
        (0.18, 0.24, 0.065, 0.72, 0.32, 0.10),
        (0.84, 0.42, 0.085, 0.85, 0.54, 0.18),
        (0.24, 0.54, 0.045, 0.30, 0.58, 0.78),
        (0.76, 0.68, 0.055, 0.78, 0.35, 0.12),
    ] {
        let d = hypot2(nx - bx, (ny - by) * (16.0 / 9.0));
        let soft = smoothstep(rad, rad * 0.45, d);
        r += soft * br * 0.36;
        g += soft * bg * 0.36;
        b += soft * bb * 0.36;
    }

    // Stylized studio portrait silhouette & warm key-lit rim (centered hero framing)
    let head_d = hypot2((nx - 0.50) * 1.05, (ny - 0.44) * 1.55);
    let torso_d = hypot2((nx - 0.50) * 0.62, (ny - 0.82) * 1.15);
    let subject = smoothstep(0.20, 0.175, head_d).max(smoothstep(0.30, 0.27, torso_d));
    if subject > 0.0 {
        // Warm realistic skin/studio-lit shading across subject
        let shade = (0.35 + (nx - 0.32) * 1.25 - (ny - 0.42) * 0.4).clamp(0.12, 1.0);
        let rim = smoothstep(0.16, 0.195, head_d) * smoothstep(0.35, 0.75, nx);
        let sr = 0.24 + shade * 0.52 + rim * 0.35;
        let sg = 0.16 + shade * 0.34 + rim * 0.22;
        let sb = 0.13 + shade * 0.23 + rim * 0.08;
        r = lerp(r, sr, subject * 0.88);
        g = lerp(g, sg, subject * 0.88);
        b = lerp(b, sb, subject * 0.88);
    }

    // Subtle S-Log3-like lifted blacks & gentle filmic rolloff
    r = (0.045 + r * 0.86).clamp(0.0, 1.0);
    g = (0.042 + g * 0.86).clamp(0.0, 1.0);
    b = (0.048 + b * 0.86).clamp(0.0, 1.0);
    (r, g, b)
}

fn sample_workflow_orbital(nx: f64, ny: f64) -> (f64, f64, f64) {
    // Deep carbon-obsidian tech B-roll with luminous orbital rings & telemetry bars
    let mut r = 0.045 + 0.04 * (1.0 - ny);
    let mut g = 0.055 + 0.05 * (1.0 - ny);
    let mut b = 0.085 + 0.07 * ny;

    // Fine architectural grid
    let gx = ((nx * 18.0).fract() - 0.5).abs();
    let gy = ((ny * 32.0).fract() - 0.5).abs();
    let grid = if gx < 0.018 || gy < 0.018 { 0.055 } else { 0.0 };
    r += grid * 0.7;
    g += grid * 0.85;
    b += grid * 1.1;

    // Luminous orbital horizon rings (Solar Orange + Ice Cyan)
    let cx = 0.5;
    let cy = 0.45;
    let dist = hypot2((nx - cx) * 1.0, (ny - cy) * (16.0 / 9.0));
    let ring1 = (-(dist - 0.26).powi(2) * 1800.0).exp();
    let ring2 = (-(dist - 0.38).powi(2) * 1100.0).exp();
    let core = (-dist * dist * 16.0).exp();

    r += ring1 * 0.92 + ring2 * 0.22 + core * 0.55;
    g += ring1 * 0.38 + ring2 * 0.72 + core * 0.24;
    b += ring1 * 0.08 + ring2 * 0.96 + core * 0.14;

    // Glowing floating timeline/waveform UI bars across lower-mid band
    for i in 0..7 {
        let bx = 0.14 + (i as f64) * 0.115;
        let bar_h = 0.05 + 0.065 * ((i as f64 * 1.37).sin().abs());
        let in_x = (nx - bx).abs() < 0.032;
        let in_y = (ny - 0.68).abs() < bar_h;
        if in_x && in_y {
            let glow = 1.0 - ((nx - bx).abs() / 0.032) * 0.35;
            let accent = if i % 2 == 0 {
                (0.95, 0.42, 0.08)
            } else {
                (0.24, 0.74, 0.96)
            };
            r = lerp(r, accent.0, 0.55 * glow);
            g = lerp(g, accent.1, 0.55 * glow);
            b = lerp(b, accent.2, 0.55 * glow);
        }
    }

    (r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0))
}

fn sample_solar_horizon_grade(nx: f64, ny: f64) -> (f64, f64, f64) {
    // Signature Solar Horizon orb rising over reflective dark monolith ground
    let horizon_y = 0.56;
    let mut r = 0.04;
    let mut g = 0.025;
    let mut b = 0.035;

    // Dramatic warm atmospheric sunset sky above horizon
    let sky_glow = (-(ny - horizon_y).powi(2) * 14.0).exp();
    r += sky_glow * 0.72;
    g += sky_glow * 0.24;
    b += sky_glow * 0.07;

    // Radiant split-horizon solar planet
    let orb_d = hypot2(nx - 0.5, (ny - 0.43) * (16.0 / 9.0));
    let orb_corona = (-orb_d * orb_d * 9.5).exp();
    r += orb_corona * 0.78;
    g += orb_corona * 0.34;
    b += orb_corona * 0.08;

    let orb_mask = smoothstep(0.225, 0.212, orb_d);
    let equator_slice = if (ny - 0.435).abs() < 0.0085 && orb_mask > 0.5 {
        1.0
    } else {
        0.0
    };
    if orb_mask > 0.0 {
        let t = ((ny - 0.28) / 0.30).clamp(0.0, 1.0);
        let or = lerp(0.98, 0.78, t);
        let og = lerp(0.76, 0.14, t);
        let ob = lerp(0.24, 0.08, t);
        r = lerp(r, or, orb_mask);
        g = lerp(g, og, orb_mask);
        b = lerp(b, ob, orb_mask);
        if equator_slice > 0.5 {
            r *= 0.08;
            g *= 0.06;
            b *= 0.06;
        }
    }

    // Reflective obsidian water/floor below horizon
    if ny > horizon_y {
        let depth = (ny - horizon_y) / (1.0 - horizon_y);
        let ripple = ((ny * 95.0).sin() * 0.5 + 0.5) * (1.0 - depth);
        let refl_x = (-(nx - 0.5).powi(2) * 18.0).exp();
        r = 0.05 + refl_x * ripple * 0.65;
        g = 0.03 + refl_x * ripple * 0.28;
        b = 0.035 + refl_x * ripple * 0.09 + depth * 0.08;
    }

    (r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0))
}

fn hypot2(x: f64, y: f64) -> f64 {
    (x * x + y * y).sqrt()
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn to_u8(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Pure-Rust PNG writer (RGBA8, zlib-compressed via `flate2`, zero external image crates).
fn write_rgba_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), MediaError> {
    let expected = (width as usize) * (height as usize) * 4;
    if rgba.len() != expected {
        return Err(MediaError::ProcessFailed(
            "invalid RGBA buffer length for PNG encoding".to_string(),
        ));
    }

    let mut raw_scanlines = Vec::with_capacity((height as usize) * (1 + (width as usize) * 4));
    let row_bytes = (width as usize) * 4;
    for y in 0..(height as usize) {
        raw_scanlines.push(0u8); // Filter type 0 (None)
        raw_scanlines.extend_from_slice(&rgba[y * row_bytes..(y + 1) * row_bytes]);
    }

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(&raw_scanlines)?;
    let idat = encoder.finish()?;

    let mut out = Vec::with_capacity(64 + idat.len());
    out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(6); // RGBA
    ihdr.push(0); // compression
    ihdr.push(0); // filter
    ihdr.push(0); // interlace
    push_png_chunk(&mut out, b"IHDR", &ihdr);
    push_png_chunk(&mut out, b"IDAT", &idat);
    push_png_chunk(&mut out, b"IEND", &[]);

    let tmp_guard = TempFileGuard::new(unique_temp_path(path, "png"));
    fs::write(tmp_guard.path(), &out)?;
    tmp_guard.commit(path)?;
    Ok(())
}

fn push_png_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);
    let mut crc: u32 = 0xffff_ffff;
    for &b in chunk_type.iter().chain(data.iter()) {
        crc ^= b as u32;
        for _ in 0..8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ 0xedb8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    out.extend_from_slice(&(!crc).to_be_bytes());
}

fn demo_music_beats(duration_sec: f64) -> Vec<f64> {
    let beat_sec: f64 = 60.0 / 118.0;
    let mut beats = Vec::new();
    let mut b_t: f64 = 0.0;
    while b_t < duration_sec - 0.1 {
        beats.push((b_t * 1000.0).round() / 1000.0);
        b_t += beat_sec;
    }
    beats
}

fn read_stereo_wav_peaks(path: &Path, buckets: usize) -> Option<Vec<f32>> {
    if buckets == 0 {
        return Some(Vec::new());
    }
    let bytes = fs::read(path).ok()?;
    if bytes.len() < 44
        || &bytes[0..4] != b"RIFF"
        || &bytes[8..12] != b"WAVE"
        || &bytes[36..40] != b"data"
    {
        return None;
    }
    let pcm = &bytes[44..];
    let num_samples = pcm.len() / 4;
    if num_samples < buckets {
        return None;
    }
    let step = (num_samples / buckets).max(1);
    let mut out = Vec::with_capacity(buckets);
    for b in 0..buckets {
        let lo = b * step;
        let hi = ((b + 1) * step).min(num_samples);
        let mut peak = 0.0f32;
        for i in lo..hi {
            let off = i * 4;
            let l = i16::from_le_bytes([pcm[off], pcm[off + 1]]);
            let r = i16::from_le_bytes([pcm[off + 2], pcm[off + 3]]);
            let lv = (i32::from(l).abs() as f32 / 32767.0).min(1.0);
            let rv = (i32::from(r).abs() as f32 / 32767.0).min(1.0);
            let v = lv.max(rv);
            if v > peak {
                peak = v;
            }
        }
        out.push((peak * 100.0).round() / 100.0);
    }
    Some(out)
}

/// Synthesizes a lush, warm 118 BPM editorial lo-fi/synth pulse stereo WAV (16.0s).
fn ensure_music_wav(dir: &Path) -> Result<(PathBuf, Vec<f32>, Vec<f64>), MediaError> {
    static MUSIC_META: OnceLock<(Vec<f32>, Vec<f64>)> = OnceLock::new();

    let path = dir.join("solaris_beat_118bpm.wav");
    let duration_sec: f64 = 16.0;

    if is_non_empty_file(&path) {
        if let Some((peaks, beats)) = MUSIC_META.get() {
            return Ok((path, peaks.clone(), beats.clone()));
        }
        if let Some(peaks) = read_stereo_wav_peaks(&path, 64) {
            let beats = demo_music_beats(duration_sec);
            let (cached_peaks, cached_beats) = MUSIC_META.get_or_init(|| (peaks, beats));
            return Ok((path, cached_peaks.clone(), cached_beats.clone()));
        }
    }

    let beat_sec: f64 = 60.0 / 118.0;
    let beats = demo_music_beats(duration_sec);

    let total_samples = (duration_sec * SAMPLE_RATE as f64) as usize;
    let mut left = vec![0.0f32; total_samples];
    let mut right = vec![0.0f32; total_samples];

    // Warm minor-9th editorial chord progression: Dm9 -> Bbmaj7 -> Gm9 -> A7sus4
    let chords: [([f64; 4], f64); 4] = [
        ([293.66, 349.23, 440.00, 523.25], 73.42), // Dm7/9 + D2
        ([233.08, 293.66, 349.23, 440.00], 58.27), // Bbmaj7 + Bb1
        ([196.00, 233.08, 293.66, 349.23], 49.00), // Gm7 + G1
        ([220.00, 293.66, 329.63, 392.00], 55.00), // A7sus4 + A1
    ];

    let mut noise_state: u32 = 0x1234_5678;

    for i in 0..total_samples {
        let t = i as f64 / SAMPLE_RATE as f64;
        let bar_idx = ((t / 4.0) as usize).min(chords.len() - 1);
        let bar_local = t % 4.0;
        let (notes, bass_hz) = chords[bar_idx];

        // Warm electric-piano/pad chord with gentle sidechain-like breathing
        let pad_env = smoothstep(0.0, 0.22, bar_local) * smoothstep(4.0, 3.65, bar_local);
        let mut pad_l = 0.0;
        let mut pad_r = 0.0;
        for (ni, &hz) in notes.iter().enumerate() {
            let detune = 1.0 + (ni as f64 - 1.5) * 0.0018;
            let ph_l = 2.0 * PI * hz * detune * t;
            let ph_r = 2.0 * PI * hz * (2.0 - detune) * t;
            pad_l += (ph_l.sin() + 0.22 * (2.0 * ph_l).sin()) * 0.075;
            pad_r += (ph_r.sin() + 0.22 * (2.0 * ph_r).sin()) * 0.075;
        }
        pad_l *= pad_env;
        pad_r *= pad_env;

        // Warm analog sub-bass pulse on beat 1 & 2.5
        let beat_pos = t / beat_sec;
        let beat_frac = beat_pos.fract();
        let beat_num = (beat_pos.floor() as usize) % 4;
        let bass_env = (-beat_frac * 2.2).exp();
        let bass = (2.0 * PI * bass_hz * t).sin() * 0.19 * (0.55 + 0.45 * bass_env);

        // Soft punchy kick on beats 0 and 2
        let kick = if beat_num == 0 || beat_num == 2 {
            let kt = beat_frac * beat_sec;
            if kt < 0.22 {
                let pitch = 46.0 + 78.0 * (-kt * 28.0).exp();
                (2.0 * PI * pitch * kt).sin() * (-kt * 14.0).exp() * 0.34
            } else {
                0.0
            }
        } else {
            0.0
        };

        // Crisp subtle hi-hat tick on eighth-note offbeats
        let eighth_frac = (beat_pos * 2.0).fract();
        let eighth_num = (beat_pos * 2.0).floor() as usize;
        noise_state = noise_state
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        let white = ((noise_state >> 16) as f64 / 32768.0) - 1.0;
        let hat = if eighth_num % 2 == 1 {
            let ht = eighth_frac * (beat_sec * 0.5);
            if ht < 0.045 {
                white * (-ht * 75.0).exp() * 0.065
            } else {
                0.0
            }
        } else {
            0.0
        };

        // Global smooth fade-in/fade-out
        let master_env =
            smoothstep(0.0, 0.25, t) * smoothstep(duration_sec, duration_sec - 0.85, t);
        let l = ((pad_l + bass + kick + hat * 0.85) * master_env).tanh() * 0.72;
        let r = ((pad_r + bass + kick + hat * 1.10) * master_env).tanh() * 0.72;
        left[i] = l as f32;
        right[i] = r as f32;
    }

    write_stereo_wav(&path, &left, &right, SAMPLE_RATE)?;
    let peaks = compute_stereo_peaks(&left, &right, 64);
    let (cached_peaks, cached_beats) = MUSIC_META.get_or_init(|| (peaks, beats));
    Ok((path, cached_peaks.clone(), cached_beats.clone()))
}

/// Synthesizes a clean, musical narration-cadence harmonic bed timed to the 5 demo caption
/// phrases (with natural breath gaps) so A1 VOICE meters & A2 sidechain ducking come alive.
fn ensure_voice_cadence_wav(dir: &Path) -> Result<(PathBuf, Vec<f32>), MediaError> {
    static VOICE_PEAKS: OnceLock<Vec<f32>> = OnceLock::new();

    let path = dir.join("narrator_cadence_vo.wav");
    if is_non_empty_file(&path) {
        if let Some(peaks) = VOICE_PEAKS.get() {
            return Ok((path, peaks.clone()));
        }
        if let Some(peaks) = read_stereo_wav_peaks(&path, 64) {
            let cached = VOICE_PEAKS.get_or_init(|| peaks);
            return Ok((path, cached.clone()));
        }
    }

    let duration_sec = 16.0;
    let total_samples = (duration_sec * SAMPLE_RATE as f64) as usize;
    let mut left = vec![0.0f32; total_samples];
    let mut right = vec![0.0f32; total_samples];

    // Phrase windows matching the 5 demo caption bursts on T1
    let phrases: [(f64, f64, [f64; 3]); 5] = [
        (0.30, 3.10, [440.00, 523.25, 659.25]),
        (3.50, 6.60, [392.00, 440.00, 587.33]),
        (7.00, 10.20, [349.23, 440.00, 523.25]),
        (10.60, 13.30, [392.00, 493.88, 587.33]),
        (13.70, 15.65, [440.00, 587.33, 659.25]),
    ];

    for i in 0..total_samples {
        let t = i as f64 / SAMPLE_RATE as f64;
        let mut sig = 0.0;
        for &(start, end, notes) in &phrases {
            if t >= start && t <= end {
                let seg_dur = end - start;
                let local = t - start;
                let note_dur = seg_dur / 3.0;
                let note_idx = ((local / note_dur) as usize).min(2);
                let nt = local - (note_idx as f64) * note_dur;
                let hz = notes[note_idx];
                // Soft, pleasant marimba/bell-like cadence pulse
                let attack = smoothstep(0.0, 0.025, nt);
                let decay = (-nt * 3.8).exp();
                let phrase_env =
                    smoothstep(0.0, 0.08, local) * smoothstep(seg_dur, seg_dur - 0.14, local);
                let tone = (2.0 * PI * hz * nt).sin() * 0.72
                    + (2.0 * PI * hz * 2.0 * nt).sin() * 0.18 * (-nt * 9.0).exp();
                sig += tone * attack * decay * phrase_env * 0.28;
            }
        }
        let s = sig.tanh() as f32;
        left[i] = s;
        right[i] = s;
    }

    write_stereo_wav(&path, &left, &right, SAMPLE_RATE)?;
    let peaks = compute_stereo_peaks(&left, &right, 64);
    let cached = VOICE_PEAKS.get_or_init(|| peaks);
    Ok((path, cached.clone()))
}

fn compute_stereo_peaks(left: &[f32], right: &[f32], buckets: usize) -> Vec<f32> {
    if left.is_empty() || buckets == 0 {
        return Vec::new();
    }
    let step = (left.len() / buckets).max(1);
    let mut out = Vec::with_capacity(buckets);
    for b in 0..buckets {
        let lo = b * step;
        let hi = ((b + 1) * step).min(left.len());
        let mut peak = 0.0f32;
        for i in lo..hi {
            let v = left[i].abs().max(right[i].abs());
            if v > peak {
                peak = v;
            }
        }
        out.push((peak * 100.0).round() / 100.0);
    }
    out
}

fn write_stereo_wav(
    path: &Path,
    left: &[f32],
    right: &[f32],
    sample_rate: u32,
) -> Result<(), MediaError> {
    let num_samples = left.len().min(right.len());
    let data_bytes = (num_samples * 4) as u32;
    let mut buf = Vec::with_capacity(44 + data_bytes as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes()); // PCM chunk size
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    buf.extend_from_slice(&2u16.to_le_bytes()); // stereo
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&(sample_rate * 4).to_le_bytes()); // byte rate
    buf.extend_from_slice(&4u16.to_le_bytes()); // block align
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_bytes.to_le_bytes());

    for i in 0..num_samples {
        let l = (left[i].clamp(-1.0, 1.0) * 32767.0).round() as i16;
        let r = (right[i].clamp(-1.0, 1.0) * 32767.0).round() as i16;
        buf.extend_from_slice(&l.to_le_bytes());
        buf.extend_from_slice(&r.to_le_bytes());
    }

    let tmp_guard = TempFileGuard::new(unique_temp_path(path, "wav"));
    fs::write(tmp_guard.path(), &buf)?;
    tmp_guard.commit(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plate_vf_uses_per_frame_eval_for_animated_eq() {
        let founder_vf = build_plate_vf(VisualScene::FounderStudio, 16.0);
        assert!(founder_vf.contains(
            "eq=brightness='0.015*sin(t*0.8)':saturation='1.0+0.04*sin(t*0.5)':eval=frame"
        ));
        let grade_vf = build_plate_vf(VisualScene::SolarHorizonGrade, 4.0);
        assert!(grade_vf.contains("eq=contrast='1.0+0.05*sin(t*0.9)':eval=frame"));
    }

    #[test]
    fn generates_demo_media_pack_without_ffmpeg() {
        let dir = std::env::temp_dir().join(format!("solaris_demo_media_{}", Uuid::new_v4()));
        let _ = fs::remove_dir_all(&dir);
        let toolchain = MediaToolchain::with_paths(None, None);
        let pack = ensure_demo_media_pack(&dir, &toolchain).expect("generate demo pack");

        assert!(pack.a_roll.path.is_file());
        assert!(pack.b_roll_workflow.path.is_file());
        assert!(pack.b_roll_grade.path.is_file());
        assert!(pack.voice_stem.path.is_file());
        assert!(pack.music_beat.path.is_file());
        assert!(!pack.music_beat.waveform_peaks.is_empty());
        assert!(!pack.music_beat.beats.is_empty());
        assert!(!pack.voice_stem.waveform_peaks.is_empty());

        // PNG magic bytes when FFmpeg is absent
        let png_bytes = fs::read(&pack.a_roll.path).expect("read png");
        assert_eq!(&png_bytes[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);

        // WAV magic bytes
        let wav_bytes = fs::read(&pack.music_beat.path).expect("read wav");
        assert_eq!(&wav_bytes[..4], b"RIFF");
        assert_eq!(&wav_bytes[8..12], b"WAVE");

        // No leaked .tmp. files
        let version_dir = dir.join("demo_media").join(DEMO_PACK_VERSION);
        for entry in fs::read_dir(&version_dir).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            assert!(!name.contains(".tmp."), "leaked temp file: {name}");
        }

        // Cached second call skips synthesis and returns identical waveform peaks & beats
        let cached = ensure_demo_media_pack(&dir, &toolchain).expect("cached demo pack");
        assert_eq!(
            cached.music_beat.waveform_peaks,
            pack.music_beat.waveform_peaks
        );
        assert_eq!(cached.music_beat.beats, pack.music_beat.beats);
        assert_eq!(
            cached.voice_stem.waveform_peaks,
            pack.voice_stem.waveform_peaks
        );

        // Direct WAV peak reader agrees with synthesized peaks within 16-bit PCM rounding tolerance
        let wav_peaks = read_stereo_wav_peaks(&pack.music_beat.path, 64).expect("wav peaks");
        assert_eq!(wav_peaks.len(), pack.music_beat.waveform_peaks.len());
        for (a, b) in wav_peaks.iter().zip(pack.music_beat.waveform_peaks.iter()) {
            assert!(
                (a - b).abs() <= 0.011,
                "wav peak {a} diverged from synthesized peak {b}"
            );
        }

        let _ = fs::remove_dir_all(&dir);
    }
}
