use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use solaris_core_ir::{ColorGrade, Transform2D};

use crate::error::MediaError;
use crate::export::{
    append_color_grade_filters, build_color_grade_core_filters, escape_ffmpeg_filter_path,
    validate_local_media_path,
};
use crate::ffmpeg::MediaToolchain;

#[derive(Debug, Clone, Default)]
pub struct CompositeFrameOptions<'a> {
    pub canvas_width: u32,
    pub canvas_height: u32,
    pub max_width: Option<u32>,
    pub transform: Option<&'a Transform2D>,
    pub grade: Option<&'a ColorGrade>,
    pub resolved_lut: Option<&'a Path>,
    pub overlay_pngs: &'a [PathBuf],
}

fn ensure_even(n: u32) -> u32 {
    if n <= 2 { 2 } else { n & !1 }
}

pub fn capture_frame_png(
    toolchain: &MediaToolchain,
    input: &Path,
    time_sec: f64,
    output_png: &Path,
    max_width: Option<u32>,
) -> Result<PathBuf, MediaError> {
    capture_graded_frame_png(
        toolchain, input, time_sec, output_png, max_width, None, None,
    )
}

pub fn build_grade_preview_vf(
    grade: Option<&ColorGrade>,
    resolved_lut: Option<&Path>,
    max_width: Option<u32>,
) -> Option<String> {
    let mut filters: Vec<String> = Vec::new();
    if let Some(w) = max_width.filter(|&w| w > 0) {
        let even_w = ensure_even(w);
        filters.push(format!("scale='min({even_w},iw)':-2"));
    }

    if let Some(g) = grade {
        let (cb, eq) = build_color_grade_core_filters(g, None);
        filters.push(cb);
        filters.push(eq);

        let lut_intensity = if g.lut_intensity.is_finite() {
            g.lut_intensity.clamp(0.0, 1.0)
        } else {
            0.85
        };
        if let Some(lut_path) = resolved_lut
            && lut_intensity > 0.001
        {
            let escaped = escape_ffmpeg_filter_path(&lut_path.to_string_lossy());
            filters.push(format!("lut3d=file={escaped}"));
        }
    }

    if filters.is_empty() {
        None
    } else {
        Some(filters.join(","))
    }
}

pub fn capture_graded_frame_png(
    toolchain: &MediaToolchain,
    input: &Path,
    time_sec: f64,
    output_png: &Path,
    max_width: Option<u32>,
    grade: Option<&ColorGrade>,
    resolved_lut: Option<&Path>,
) -> Result<PathBuf, MediaError> {
    let ffmpeg = toolchain
        .ffmpeg_path
        .as_ref()
        .ok_or_else(|| MediaError::ToolchainUnavailable("ffmpeg not found".to_string()))?;

    if !time_sec.is_finite() || time_sec < 0.0 {
        return Err(MediaError::ParseError(
            "time_sec must be finite and >= 0".to_string(),
        ));
    }
    if let Some(lp) = resolved_lut {
        validate_local_media_path(&lp.to_string_lossy())?;
    }
    if let Some(parent) = output_png.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }

    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-y", "-hide_banner", "-v", "error", "-ss"])
        .arg(format!("{time_sec:.3}"))
        .arg("-i")
        .arg(input);

    if let Some(g) = grade {
        let mut fc: Vec<String> = Vec::new();
        let mut cur = "[0:v]".to_string();
        if let Some(w) = max_width.filter(|&w| w > 0) {
            let even_w = ensure_even(w);
            fc.push(format!("{cur}scale='min({even_w},iw)':-2[vscaled]"));
            cur = "[vscaled]".to_string();
        }
        append_color_grade_filters(
            &mut fc,
            &cur,
            "[vout]",
            "preview",
            g,
            resolved_lut,
            false,
            None,
        );
        cmd.args(["-filter_complex", &fc.join(";"), "-map", "[vout]"]);
    } else if let Some(w) = max_width.filter(|&w| w > 0) {
        let even_w = ensure_even(w);
        cmd.args(["-vf", &format!("scale='min({even_w},iw)':-2")]);
    }

    cmd.args(["-frames:v", "1"]).arg(output_png);

    let out = cmd.output()?;
    if !out.status.success() {
        return Err(MediaError::ProcessFailed(
            String::from_utf8_lossy(&out.stderr).to_string(),
        ));
    }

    let valid_png = output_png
        .metadata()
        .is_ok_and(|m| m.is_file() && m.len() > 0);
    if !valid_png {
        return Err(MediaError::ProcessFailed(format!(
            "ffmpeg exited 0 but failed to write non-empty PNG at '{}' (timestamp {time_sec:.3}s may exceed media duration)",
            output_png.display()
        )));
    }

    Ok(output_png.to_path_buf())
}

pub fn capture_composited_frame_png(
    toolchain: &MediaToolchain,
    input: &Path,
    time_sec: f64,
    output_png: &Path,
    opts: &CompositeFrameOptions<'_>,
) -> Result<(PathBuf, u32, u32), MediaError> {
    let ffmpeg = toolchain
        .ffmpeg_path
        .as_ref()
        .ok_or_else(|| MediaError::ToolchainUnavailable("ffmpeg not found".to_string()))?;

    if !time_sec.is_finite() || time_sec < 0.0 {
        return Err(MediaError::ParseError(
            "time_sec must be finite and >= 0".to_string(),
        ));
    }
    if let Some(lp) = opts.resolved_lut {
        validate_local_media_path(&lp.to_string_lossy())?;
    }
    if let Some(parent) = output_png.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }

    let canvas_w = ensure_even(if opts.canvas_width >= 2 {
        opts.canvas_width
    } else {
        1080
    });
    let canvas_h = ensure_even(if opts.canvas_height >= 2 {
        opts.canvas_height
    } else {
        1920
    });
    let (out_w, out_h) = if let Some(mw) = opts.max_width.filter(|&w| w >= 2 && w < canvas_w) {
        let target_w = ensure_even(mw);
        let scaled_h = ((canvas_h as f64) * (target_w as f64 / canvas_w as f64)).round() as u32;
        (target_w, ensure_even(scaled_h.max(2)))
    } else {
        (canvas_w, canvas_h)
    };

    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-y", "-hide_banner", "-v", "error"]);
    if time_sec > 0.001 {
        cmd.arg("-ss").arg(format!("{time_sec:.3}"));
    }
    cmd.arg("-i").arg(input);

    let active_overlays: Vec<&PathBuf> = opts.overlay_pngs.iter().filter(|p| p.is_file()).collect();
    for ov in &active_overlays {
        cmd.arg("-i").arg(ov);
    }

    let filter_complex = build_composite_frame_filter_complex(
        out_w,
        out_h,
        opts.transform,
        opts.grade,
        opts.resolved_lut,
        active_overlays.len(),
    );

    cmd.args([
        "-filter_complex",
        &filter_complex,
        "-map",
        "[vfinal]",
        "-frames:v",
        "1",
    ])
    .arg(output_png);

    let out = cmd.output()?;
    if !out.status.success() {
        return Err(MediaError::ProcessFailed(
            String::from_utf8_lossy(&out.stderr).to_string(),
        ));
    }

    let (png_w, png_h) = read_png_dimensions(output_png)?;
    Ok((output_png.to_path_buf(), png_w, png_h))
}

fn build_composite_frame_filter_complex(
    out_w: u32,
    out_h: u32,
    transform: Option<&Transform2D>,
    grade: Option<&ColorGrade>,
    resolved_lut: Option<&Path>,
    overlay_count: usize,
) -> String {
    let t = transform.cloned().unwrap_or_default();
    let scale = if t.scale.is_finite() {
        t.scale.clamp(0.1, 4.0)
    } else {
        1.0
    };
    let scaled_w = ensure_even(((out_w as f64 * scale).round() as u32).max(2));
    let scaled_h = ensure_even(((out_h as f64 * scale).round() as u32).max(2));
    let ox = if t.x.is_finite() {
        (t.x * out_w as f64).round() as i32
    } else {
        0
    };
    let oy = if t.y.is_finite() {
        (t.y * out_h as f64).round() as i32
    } else {
        0
    };

    let reframe_prefix = if let Some(rf) = &t.smart_reframe
        && rf.x.is_finite()
        && rf.y.is_finite()
        && rf.width.is_finite()
        && rf.height.is_finite()
        && rf.width > 0.05
        && rf.height > 0.05
    {
        let rx = rf.x.clamp(0.0, 0.95);
        let ry = rf.y.clamp(0.0, 0.95);
        let rw = rf.width.clamp(0.05, 1.0 - rx);
        let rh = rf.height.clamp(0.05, 1.0 - ry);
        format!("crop=iw*{rw:.4}:ih*{rh:.4}:iw*{rx:.4}:ih*{ry:.4},")
    } else {
        String::new()
    };

    let (scale_crop_or_pad, overlay_ox, overlay_oy) = if scale >= 1.0 {
        let crop_expr = if ox == 0 && oy == 0 {
            format!("crop={out_w}:{out_h}")
        } else {
            format!(
                "crop={out_w}:{out_h}:clamp((iw-{out_w})/2-({ox}),0,iw-{out_w}):clamp((ih-{out_h})/2-({oy}),0,ih-{out_h})"
            )
        };
        (
            format!(
                "{reframe_prefix}scale={scaled_w}:{scaled_h}:force_original_aspect_ratio=increase,{crop_expr}"
            ),
            0,
            0,
        )
    } else {
        (
            format!(
                "{reframe_prefix}scale={scaled_w}:{scaled_h}:force_original_aspect_ratio=decrease,format=rgba,pad={out_w}:{out_h}:(ow-iw)/2:(oh-ih)/2:color=black@0.0"
            ),
            ox,
            oy,
        )
    };

    let rotation_suffix = if t.rotation_deg.is_finite() && t.rotation_deg.abs() > 0.05 {
        let rad = t.rotation_deg.to_radians();
        format!(",format=rgba,rotate={rad:.6}:c=none:ow=rotw({rad:.6}):oh=roth({rad:.6})")
    } else {
        String::new()
    };

    let opacity_suffix = if t.opacity.is_finite() && t.opacity < 0.999 {
        format!(
            ",format=rgba,colorchannelmixer=aa={:.3}",
            t.opacity.clamp(0.0, 1.0)
        )
    } else {
        String::new()
    };

    let pos_x = if overlay_ox == 0 {
        "(W-w)/2".to_string()
    } else {
        format!("(W-w)/2+({overlay_ox})")
    };
    let pos_y = if overlay_oy == 0 {
        "(H-h)/2".to_string()
    } else {
        format!("(H-h)/2+({overlay_oy})")
    };

    let mut filters: Vec<String> = Vec::new();
    filters.push(format!("color=c=black:s={out_w}x{out_h}:d=1[base]"));
    filters.push(format!(
        "[0:v]{scale_crop_or_pad}{rotation_suffix}{opacity_suffix}[vclip]"
    ));
    filters.push(format!(
        "[base][vclip]overlay={pos_x}:{pos_y}:eof_action=pass[vcomp]"
    ));

    let mut cur = "[vcomp]".to_string();
    if let Some(g) = grade {
        append_color_grade_filters(
            &mut filters,
            &cur,
            "[vgraded]",
            "qa",
            g,
            resolved_lut,
            false,
            None,
        );
        cur = "[vgraded]".to_string();
    }

    for idx in 0..overlay_count {
        let in_idx = idx + 1;
        let scaled_tag = format!("[ov_s{idx}]");
        let next_tag = format!("[vov{idx}]");
        filters.push(format!(
            "[{in_idx}:v]scale={out_w}:{out_h}:flags=lanczos,format=rgba{scaled_tag};{cur}{scaled_tag}overlay=0:0:eof_action=repeat{next_tag}"
        ));
        cur = next_tag;
    }

    filters.push(format!("{cur}null[vfinal]"));
    filters.join(";")
}

fn read_png_dimensions(path: &Path) -> Result<(u32, u32), MediaError> {
    let mut file = std::fs::File::open(path)?;
    let mut header = [0u8; 24];
    file.read_exact(&mut header).map_err(|e| {
        MediaError::ParseError(format!(
            "failed to read PNG header from '{}': {e}",
            path.display()
        ))
    })?;
    const PNG_SIG: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
    if header[..8] != PNG_SIG || &header[12..16] != b"IHDR" {
        return Err(MediaError::ParseError(format!(
            "'{}' is not a valid PNG file",
            path.display()
        )));
    }
    let w = u32::from_be_bytes([header[16], header[17], header[18], header[19]]);
    let h = u32::from_be_bytes([header[20], header[21], header[22], header[23]]);
    if w == 0 || h == 0 {
        return Err(MediaError::ParseError(format!(
            "PNG '{}' has invalid dimensions {w}x{h}",
            path.display()
        )));
    }
    Ok((w, h))
}

pub fn extract_keyframes_png(
    toolchain: &MediaToolchain,
    input: &Path,
    timestamps_sec: &[f64],
    out_dir: &Path,
    max_width: u32,
) -> Result<Vec<(f64, PathBuf)>, MediaError> {
    if !out_dir.as_os_str().is_empty() {
        std::fs::create_dir_all(out_dir)?;
    }
    let mut results = Vec::new();
    let mut last_err: Option<MediaError> = None;
    for (idx, &ts) in timestamps_sec.iter().take(8).enumerate() {
        let clamped = if ts.is_finite() { ts.max(0.0) } else { 0.0 };
        let path = out_dir.join(format!("kf_{idx:02}.png"));
        match capture_frame_png(
            toolchain,
            input,
            clamped,
            &path,
            (max_width > 0).then_some(max_width),
        ) {
            Ok(written) => results.push((clamped, written)),
            Err(err) => {
                tracing::debug!(
                    time_sec = clamped,
                    error = %err,
                    "skipping keyframe timestamp that failed capture"
                );
                let _ = std::fs::remove_file(&path);
                last_err = Some(err);
            }
        }
    }
    if results.is_empty() && !timestamps_sec.is_empty() {
        return Err(last_err.unwrap_or_else(|| {
            MediaError::ProcessFailed("no keyframes could be extracted".to_string())
        }));
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use solaris_core_ir::SmartReframeBox;

    #[test]
    fn builds_grade_preview_vf_matching_export_math_and_unquoted_lut() {
        let grade = ColorGrade {
            exposure: 1.0,
            temp: 0.5,
            tint: -0.25,
            contrast: 1.1,
            saturation: 1.2,
            lut_intensity: 0.75,
            ..ColorGrade::default()
        };
        let vf = build_grade_preview_vf(
            Some(&grade),
            Some(Path::new("/tmp/o'reilly/look:1,2[a].cube")),
            Some(640),
        )
        .unwrap();
        assert!(vf.contains("scale='min(640,iw)':-2"));
        assert!(vf.contains("brightness=0.150"));
        assert!(vf.contains("rm=0.090"));
        assert!(vf.contains("gm=-0.040"));
        assert!(vf.contains("bm=-0.090"));
        assert!(vf.contains("lut3d=file=/tmp/o\\\\\\'reilly/look\\\\:1\\,2\\[a\\].cube"));
        assert!(!vf.contains("lut3d=file='"));
    }

    #[test]
    fn builds_composited_frame_filtergraph_with_transform_grade_lut_blend_and_overlays() {
        let transform = Transform2D {
            x: 0.1,
            y: -0.05,
            scale: 1.25,
            rotation_deg: 2.0,
            opacity: 0.9,
            smart_reframe: Some(SmartReframeBox {
                x: 0.2,
                y: 0.0,
                width: 0.6,
                height: 1.0,
            }),
        };
        let grade = ColorGrade {
            exposure: 0.5,
            lut_intensity: 0.5,
            ..ColorGrade::default()
        };
        let fc = build_composite_frame_filter_complex(
            540,
            960,
            Some(&transform),
            Some(&grade),
            Some(Path::new("/tmp/warm.cube")),
            2,
        );
        assert!(fc.contains("color=c=black:s=540x960:d=1[base]"));
        assert!(fc.contains("crop=iw*0.6000:ih*1.0000"));
        assert!(fc.contains("clamp((iw-540)/2-(54),0,iw-540)"));
        assert!(fc.contains("blend=all_expr='A*0.500+B*0.500'"));
        assert!(fc.contains("[1:v]scale=540:960:flags=lanczos,format=rgba[ov_s0]"));
        assert!(fc.contains("[2:v]scale=540:960:flags=lanczos,format=rgba[ov_s1]"));
        assert!(fc.ends_with("[vov1]null[vfinal]"));
    }

    #[test]
    fn parses_png_ihdr_dimensions() {
        let dir = std::env::temp_dir().join(format!("solaris_png_ihdr_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("tiny.png");
        let mut bytes = vec![137, 80, 78, 71, 13, 10, 26, 10]; // PNG sig
        bytes.extend_from_slice(&13u32.to_be_bytes()); // IHDR len
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&540u32.to_be_bytes());
        bytes.extend_from_slice(&960u32.to_be_bytes());
        std::fs::write(&p, &bytes).unwrap();
        assert_eq!(read_png_dimensions(&p).unwrap(), (540, 960));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
