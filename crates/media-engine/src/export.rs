use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use solaris_core_ir::{
    Clip, ClipKind, ColorGrade, ExportFormat, ExportQuality, ExportRequest, MediaAsset, MediaKind,
    Project, TrackKind,
};
use uuid::Uuid;

use crate::error::MediaError;
use crate::ffmpeg::MediaToolchain;
use crate::gfx_render::OverlayImageSpec;

struct TempFileGuard(PathBuf);

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportFfmpegPlan {
    pub inputs: Vec<String>,
    pub filter_complex: String,
    pub video_map: String,
    pub audio_map: Option<String>,
    pub codec_args: Vec<String>,
    pub output_path: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct ExportPlanOptions<'a> {
    pub overlay_sequence_dir: Option<&'a Path>,
    pub overlay_images: &'a [OverlayImageSpec],
    pub resolved_lut_path: Option<&'a Path>,
}

fn sanitize_f64(val: f64, default: f64, min: f64, max: f64) -> f64 {
    if val.is_finite() {
        val.clamp(min, max)
    } else {
        default
    }
}

pub fn build_export_plan(
    project: &Project,
    req: &ExportRequest,
    output_path: &Path,
    overlay_sequence_dir: Option<&Path>,
    hw_encoders: &[String],
) -> Result<ExportFfmpegPlan, MediaError> {
    build_export_plan_with_options(
        project,
        req,
        output_path,
        ExportPlanOptions {
            overlay_sequence_dir,
            overlay_images: &[],
            resolved_lut_path: project.active_grade.lut_path.as_deref().map(Path::new),
        },
        hw_encoders,
    )
}

pub fn build_export_plan_with_options(
    project: &Project,
    req: &ExportRequest,
    output_path: &Path,
    options: ExportPlanOptions<'_>,
    hw_encoders: &[String],
) -> Result<ExportFfmpegPlan, MediaError> {
    if let Some(lut_path) = options.resolved_lut_path {
        validate_local_media_path(&lut_path.to_string_lossy())?;
    }

    let w = project.canvas.width.max(2);
    let h = project.canvas.height.max(2);
    let fps_num = project.canvas.fps_num.max(1);
    let fps_den = project.canvas.fps_den.max(1);
    let fps_expr = format!("{fps_num}/{fps_den}");
    let fps_float = (fps_num as f64 / fps_den as f64).clamp(1.0, 240.0);
    let duration = project.duration_sec().max(1.0);
    let target_lufs = sanitize_f64(req.target_lufs, -14.0, -70.0, -5.0);

    let media_map: HashMap<&str, &MediaAsset> = project
        .media_pool
        .iter()
        .map(|m| (m.id.as_str(), m))
        .collect();

    let mut inputs: Vec<String> = Vec::new();
    let mut input_count: usize = 0;
    let mut filters: Vec<String> = Vec::new();

    let is_alpha_format = matches!(
        req.format,
        ExportFormat::MovProRes4444 | ExportFormat::WebmAlpha
    );
    let ov_fmt = if is_alpha_format {
        ":format=rgb"
    } else {
        ":format=auto"
    };

    if is_alpha_format {
        filters.push(format!(
            "color=c=black@0.0:s={w}x{h}:r={fps_expr}:d={duration:.3},format=rgba[base0]"
        ));
    } else {
        filters.push(format!(
            "color=c=black:s={w}x{h}:r={fps_expr}:d={duration:.3}[base0]"
        ));
    }
    let mut current_v = "[base0]".to_string();
    let mut v_idx = 0usize;
    let mut a_roll_audio_sources: Vec<(usize, &Clip, bool)> = Vec::new();
    let mut b_roll_audio_sources: Vec<(usize, &Clip, bool)> = Vec::new();

    // Respect track.solo across visual tracks (V1, V2)
    let any_visual_solo = project
        .tracks
        .iter()
        .any(|t| matches!(t.kind, TrackKind::ARoll | TrackKind::BRoll) && t.solo);

    // Visual tracks: V1 (A-Roll) then V2 (B-Roll)
    for track_kind in [TrackKind::ARoll, TrackKind::BRoll] {
        let Some(track) = project.tracks.iter().find(|t| t.kind == track_kind) else {
            continue;
        };
        if track.muted || (any_visual_solo && !track.solo) {
            continue;
        }
        for clip in &track.clips {
            let ClipKind::Media {
                media_id,
                ken_burns,
            } = &clip.payload
            else {
                continue;
            };
            let Some(asset) = media_map.get(media_id.as_str()) else {
                continue;
            };
            // Validate stream kind before mapping [idx:v]
            if !matches!(asset.kind, MediaKind::Video | MediaKind::Image) {
                continue;
            }
            validate_local_media_path(&asset.path)?;

            let input_idx = input_count;
            input_count += 1;

            let mut pre_trimmed = false;
            if asset.kind == MediaKind::Image {
                if *ken_burns {
                    inputs.extend(["-i".into(), asset.path.clone()]);
                } else {
                    inputs.extend([
                        "-loop".into(),
                        "1".into(),
                        "-t".into(),
                        format!("{:.3}", clip.duration_sec),
                        "-i".into(),
                        asset.path.clone(),
                    ]);
                }
            } else {
                // Fast input seeking (-ss / -t before -i) avoids O(N) full-file decoding on split clips.
                if clip.source_in_sec > 0.001 {
                    inputs.extend([
                        "-ss".into(),
                        format!("{:.3}", clip.source_in_sec),
                        "-t".into(),
                        format!("{:.3}", clip.duration_sec),
                        "-i".into(),
                        asset.path.clone(),
                    ]);
                    pre_trimmed = true;
                } else {
                    inputs.extend(["-i".into(), asset.path.clone()]);
                }
                let gain_db = sanitize_f64(clip.audio.gain_db, 0.0, -120.0, 36.0);
                let has_audio_stream = asset.intelligence.has_audio
                    || !asset.intelligence.waveform_peaks.is_empty()
                    || asset.intelligence.integrated_lufs.is_some();
                if track_kind == TrackKind::ARoll && gain_db > -80.0 && has_audio_stream {
                    a_roll_audio_sources.push((input_idx, clip, pre_trimmed));
                } else if track_kind == TrackKind::BRoll && gain_db > -80.0 && has_audio_stream {
                    b_roll_audio_sources.push((input_idx, clip, pre_trimmed));
                }
            }

            v_idx += 1;
            let clip_tag = format!("vc{v_idx}");
            let next_v = format!("[v{v_idx}]");

            let scale = sanitize_f64(clip.transform.scale, 1.0, 0.1, 4.0);
            let scaled_w = ensure_even(((w as f64 * scale).round() as u32).max(2));
            let scaled_h = ensure_even(((h as f64 * scale).round() as u32).max(2));
            let ox = if clip.transform.x.is_finite() {
                (clip.transform.x * w as f64).round() as i32
            } else {
                0
            };
            let oy = if clip.transform.y.is_finite() {
                (clip.transform.y * h as f64).round() as i32
            } else {
                0
            };

            let reframe_prefix = if let Some(rf) = &clip.transform.smart_reframe
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
                    format!("crop={w}:{h}")
                } else {
                    format!(
                        "crop={w}:{h}:clamp((iw-{w})/2-({ox}),0,iw-{w}):clamp((ih-{h})/2-({oy}),0,ih-{h})"
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
                // Convert to RGBA before transparent padding so PiP clips never paint solid black over V1
                (
                    format!(
                        "{reframe_prefix}scale={scaled_w}:{scaled_h}:force_original_aspect_ratio=decrease,format=rgba,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=black@0.0"
                    ),
                    ox,
                    oy,
                )
            };

            let rotation_suffix = if clip.transform.rotation_deg.is_finite()
                && clip.transform.rotation_deg.abs() > 0.05
            {
                let rad = clip.transform.rotation_deg.to_radians();
                format!(",format=rgba,rotate={rad:.6}:c=none:ow=rotw({rad:.6}):oh=roth({rad:.6})")
            } else {
                String::new()
            };

            let opacity_suffix =
                if clip.transform.opacity.is_finite() && clip.transform.opacity < 0.999 {
                    format!(
                        ",format=rgba,colorchannelmixer=aa={:.3}",
                        clip.transform.opacity.clamp(0.0, 1.0)
                    )
                } else {
                    String::new()
                };

            let mut chain = if asset.kind == MediaKind::Image && *ken_burns {
                let total_frames = (clip.duration_sec * fps_float).max(1.0).round() as u32;
                let zoom_step = 0.12 / (total_frames.max(1) as f64);
                let kb_w = ensure_even(w.saturating_mul(2));
                let kb_h = ensure_even(h.saturating_mul(2));
                let kb_pre = if scale >= 1.0 {
                    let kb_sw = ensure_even(((kb_w as f64 * scale).round() as u32).max(kb_w));
                    let kb_sh = ensure_even(((kb_h as f64 * scale).round() as u32).max(kb_h));
                    let kb_ox = ox.saturating_mul(2);
                    let kb_oy = oy.saturating_mul(2);
                    let kb_crop = if kb_ox == 0 && kb_oy == 0 {
                        format!("crop={kb_w}:{kb_h}")
                    } else {
                        format!(
                            "crop={kb_w}:{kb_h}:clamp((iw-{kb_w})/2-({kb_ox}),0,iw-{kb_w}):clamp((ih-{kb_h})/2-({kb_oy}),0,ih-{kb_h})"
                        )
                    };
                    format!(
                        "{reframe_prefix}scale={kb_sw}:{kb_sh}:force_original_aspect_ratio=increase,{kb_crop}"
                    )
                } else {
                    format!("{scale_crop_or_pad},scale={kb_w}:{kb_h}:flags=lanczos")
                };
                format!(
                    "[{input_idx}:v]{kb_pre},zoompan=z='min(zoom+{zoom_step:.6},1.12)':x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)':d={total_frames}:s={w}x{h}:fps={fps_float:.3},setpts=PTS-STARTPTS+{:.3}/TB{rotation_suffix}{opacity_suffix}[{clip_tag}]",
                    clip.start_sec
                )
            } else if asset.kind == MediaKind::Image {
                format!(
                    "[{input_idx}:v]{scale_crop_or_pad},fps={fps_float:.3},setpts=PTS-STARTPTS+{:.3}/TB{rotation_suffix}{opacity_suffix}[{clip_tag}]",
                    clip.start_sec
                )
            } else if pre_trimmed {
                format!(
                    "[{input_idx}:v]trim=duration={:.3},setpts=PTS-STARTPTS+{:.3}/TB,{scale_crop_or_pad}{rotation_suffix}{opacity_suffix}[{clip_tag}]",
                    clip.duration_sec, clip.start_sec
                )
            } else {
                format!(
                    "[{input_idx}:v]trim=start={:.3}:duration={:.3},setpts=PTS-STARTPTS+{:.3}/TB,{scale_crop_or_pad}{rotation_suffix}{opacity_suffix}[{clip_tag}]",
                    clip.source_in_sec, clip.duration_sec, clip.start_sec
                )
            };

            let end_sec = clip.start_sec + clip.duration_sec;
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
            chain.push_str(&format!(
                ";{current_v}[{clip_tag}]overlay={pos_x}:{pos_y}:enable='between(t,{:.3},{:.3})':eof_action=pass{ov_fmt}{next_v}",
                clip.start_sec, end_sec
            ));
            filters.push(chain);
            current_v = next_v;
        }
    }

    // Master Color Grade (preserving alpha via alphaextract/alphamerge on alpha exports)
    let grade_tag = "[vgraded]".to_string();
    append_color_grade_filters(
        &mut filters,
        &current_v,
        &grade_tag,
        "master",
        &project.active_grade,
        options.resolved_lut_path,
        is_alpha_format,
        None,
    );
    current_v = grade_tag;

    // FX Adjustment-Layer Grade Clips (`TrackKind::Grade`) applied inline with `enable='between(...)'`
    // directly on the main video stream (avoiding split+overlay alpha doubling on transparent exports).
    if let Some(fx_track) = project.tracks.iter().find(|t| t.kind == TrackKind::Grade)
        && !fx_track.muted
    {
        for (fx_idx, clip) in fx_track.clips.iter().enumerate() {
            let ClipKind::Grade { grade } = &clip.payload else {
                continue;
            };
            let clip_lut = grade
                .lut_path
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .map(Path::new);
            if let Some(lp) = clip_lut {
                validate_local_media_path(&lp.to_string_lossy())?;
            }
            let end_sec = clip.start_sec + clip.duration_sec;
            let enable_expr = format!("between(t,{:.3},{:.3})", clip.start_sec, end_sec);
            let next_v = format!("[vfx_done_{fx_idx}]");
            append_color_grade_filters(
                &mut filters,
                &current_v,
                &next_v,
                &format!("fx{fx_idx}"),
                grade,
                clip_lut,
                is_alpha_format,
                Some(&enable_expr),
            );
            current_v = next_v;
        }
    }

    // Tier-2 RGBA Overlay Sequence (if directory of PNG frames is supplied)
    if let Some(seq_dir) = options.overlay_sequence_dir {
        let pattern = seq_dir.join("frame_%06d.png");
        let ov_idx = input_count;
        input_count += 1;
        inputs.extend([
            "-framerate".into(),
            format!("{fps_float:.3}"),
            "-i".into(),
            pattern.to_string_lossy().to_string(),
        ]);
        filters.push(format!(
            "{current_v}[{ov_idx}:v]overlay=0:0:eof_action=pass{ov_fmt}[vseq]"
        ));
        current_v = "[vseq]".to_string();
    }

    // Tier-2 Per-Clip RGBA Overlay PNGs / Sequences (from Headless Chromium for G1/T1 clips)
    for (idx, ov) in options.overlay_images.iter().enumerate() {
        let ov_idx = input_count;
        input_count += 1;
        let next_tag = format!("[vov{idx}]");
        if ov.is_sequence {
            let ov_fps = sanitize_f64(ov.fps.unwrap_or(fps_float), fps_float, 0.001, 60.0);
            inputs.extend([
                "-framerate".into(),
                format!("{ov_fps:.3}"),
                "-i".into(),
                ov.path.to_string_lossy().to_string(),
            ]);
            let shifted = format!("[vov_s{idx}]");
            filters.push(format!(
                "[{ov_idx}:v]setpts=PTS-STARTPTS+{:.3}/TB{shifted};{current_v}{shifted}overlay=0:0:enable='between(t,{:.3},{:.3})':eof_action=repeat{ov_fmt}{next_tag}",
                ov.start_sec, ov.start_sec, ov.end_sec
            ));
        } else {
            inputs.extend(["-i".into(), ov.path.to_string_lossy().to_string()]);
            // eof_action=repeat keeps single-frame PNG visible across its entire [start_sec, end_sec] window
            filters.push(format!(
                "{current_v}[{ov_idx}:v]overlay=0:0:enable='between(t,{:.3},{:.3})':eof_action=repeat{ov_fmt}{next_tag}",
                ov.start_sec, ov.end_sec
            ));
        }
        current_v = next_tag;
    }

    // Audio Mix (A1 VOICE + A2 MUSIC [ducked vs unducked SFX] + optional V1 embedded audio with track.solo & -14 LUFS @ 48 kHz)
    let mut voice_labels: Vec<String> = Vec::new();
    let mut ducked_music_labels: Vec<String> = Vec::new();
    let mut unducked_music_labels: Vec<String> = Vec::new();
    let mut duck_target_db: Option<f64> = None;
    let mut a_idx = 0usize;

    let any_audio_solo = project.tracks.iter().any(|t| {
        matches!(
            t.kind,
            TrackKind::ARoll | TrackKind::Voice | TrackKind::Music
        ) && t.solo
    });
    let a_roll_track = project.tracks.iter().find(|t| t.kind == TrackKind::ARoll);
    let voice_track = project.tracks.iter().find(|t| t.kind == TrackKind::Voice);
    let voice_muted = voice_track.is_some_and(|t| t.muted);
    let voice_clips: &[Clip] = voice_track.map(|t| t.clips.as_slice()).unwrap_or(&[]);

    // Include V1 embedded audio whenever V1 is unmuted and solo permits, skipping individual V1
    // clips whose timeline span is substantially overlapped (>= 50%) by an explicit A1 clip.
    if !a_roll_track.is_some_and(|t| t.muted)
        && (!any_audio_solo
            || a_roll_track.is_some_and(|t| t.solo)
            || voice_track.is_some_and(|t| t.solo))
    {
        for (aroll_input_idx, clip, pre_trimmed) in a_roll_audio_sources {
            let clip_start = clip.start_sec;
            let clip_end = clip.start_sec + clip.duration_sec;
            let overlapped_by_a1 = voice_clips.iter().any(|vc| {
                let ov = (clip_end.min(vc.start_sec + vc.duration_sec)
                    - clip_start.max(vc.start_sec))
                .max(0.0);
                ov >= clip.duration_sec * 0.5
            });
            if overlapped_by_a1 && (voice_muted || !voice_clips.is_empty()) {
                continue;
            }
            a_idx += 1;
            let tag = format!("[a{a_idx}]");
            filters.push(build_audio_clip_chain(
                aroll_input_idx,
                clip,
                &tag,
                pre_trimmed,
            ));
            voice_labels.push(tag);
        }
    }

    // Include unmuted V2 (B-Roll) video clips with audio (gain_db > -80.0).
    let b_roll_track = project.tracks.iter().find(|t| t.kind == TrackKind::BRoll);
    if !b_roll_track.is_some_and(|t| t.muted)
        && (!any_audio_solo || b_roll_track.is_some_and(|t| t.solo))
    {
        for (broll_input_idx, clip, pre_trimmed) in b_roll_audio_sources {
            a_idx += 1;
            let tag = format!("[a{a_idx}]");
            filters.push(build_audio_clip_chain(
                broll_input_idx,
                clip,
                &tag,
                pre_trimmed,
            ));
            unducked_music_labels.push(tag);
        }
    }

    for (kind, is_voice) in [(TrackKind::Voice, true), (TrackKind::Music, false)] {
        let Some(track) = project.tracks.iter().find(|t| t.kind == kind) else {
            continue;
        };
        if track.muted || (any_audio_solo && !track.solo) {
            continue;
        }
        for clip in &track.clips {
            let Some(path) = media_path_for_audio(clip, &media_map) else {
                continue;
            };
            validate_local_media_path(path)?;
            let input_idx = input_count;
            input_count += 1;
            let pre_trimmed = if clip.source_in_sec > 0.001 {
                inputs.extend([
                    "-ss".into(),
                    format!("{:.3}", clip.source_in_sec),
                    "-t".into(),
                    format!("{:.3}", clip.duration_sec),
                    "-i".into(),
                    path.to_string(),
                ]);
                true
            } else {
                inputs.extend(["-i".into(), path.to_string()]);
                false
            };
            a_idx += 1;
            let tag = format!("[a{a_idx}]");
            filters.push(build_audio_clip_chain(input_idx, clip, &tag, pre_trimmed));
            if is_voice {
                voice_labels.push(tag);
            } else if clip.audio.ducking_enabled {
                let target = sanitize_f64(clip.audio.duck_target_db, -18.0, -60.0, 0.0);
                duck_target_db = Some(duck_target_db.map_or(target, |cur| cur.min(target)));
                ducked_music_labels.push(tag);
            } else {
                unducked_music_labels.push(tag);
            }
        }
    }

    let audio_map = if !voice_labels.is_empty() && !ducked_music_labels.is_empty() {
        let v_mix = if voice_labels.len() == 1 {
            voice_labels[0].clone()
        } else {
            let joined = voice_labels.join("");
            filters.push(format!(
                "{joined}amix=inputs={}:normalize=0[avoice]",
                voice_labels.len()
            ));
            "[avoice]".to_string()
        };
        let dm_mix = if ducked_music_labels.len() == 1 {
            ducked_music_labels[0].clone()
        } else {
            let joined = ducked_music_labels.join("");
            filters.push(format!(
                "{joined}amix=inputs={}:normalize=0[amusic]",
                ducked_music_labels.len()
            ));
            "[amusic]".to_string()
        };

        let ratio = ((-duck_target_db.unwrap_or(-18.0)) / 3.0).clamp(2.0, 12.0);
        let mut final_inputs = vec!["[vmain]".to_string(), "[mducked]".to_string()];
        final_inputs.extend(unducked_music_labels);
        let final_joined = final_inputs.join("");
        // Pad sidechain key [vkey] with apad=whole_dur so A2 music never truncates when voice ends early.
        // Append aresample=48000 after loudnorm so FFmpeg never upsamples AAC output to 96 kHz.
        filters.push(format!(
            "{v_mix}asplit=2[vkey_raw][vmain];[vkey_raw]apad=whole_dur={duration:.3}[vkey];{dm_mix}[vkey]sidechaincompress=threshold=0.03:ratio={ratio:.1}:attack=25:release=250[mducked];{final_joined}amix=inputs={}:normalize=0,loudnorm=I={target_lufs:.1}:TP=-1.5:LRA=11,aresample=48000[afinal]",
            final_inputs.len()
        ));
        Some("[afinal]".to_string())
    } else {
        let all: Vec<String> = voice_labels
            .into_iter()
            .chain(ducked_music_labels)
            .chain(unducked_music_labels)
            .collect();
        if all.is_empty() {
            None
        } else if all.len() == 1 {
            filters.push(format!(
                "{}loudnorm=I={target_lufs:.1}:TP=-1.5:LRA=11,aresample=48000[afinal]",
                all[0]
            ));
            Some("[afinal]".to_string())
        } else {
            let joined = all.join("");
            filters.push(format!(
                "{joined}amix=inputs={}:normalize=0,loudnorm=I={target_lufs:.1}:TP=-1.5:LRA=11,aresample=48000[afinal]",
                all.len()
            ));
            Some("[afinal]".to_string())
        }
    };

    let codec_args = codec_args_for(req.format, req.quality, hw_encoders);

    Ok(ExportFfmpegPlan {
        inputs,
        filter_complex: filters.join(";"),
        video_map: current_v,
        audio_map,
        codec_args,
        output_path: output_path.to_path_buf(),
    })
}

/// Shared normalized colorbalance + eq filter pair used by both export and preview/QA frames.
pub fn build_color_grade_core_filters(
    g: &ColorGrade,
    enable_expr: Option<&str>,
) -> (String, String) {
    let exposure = sanitize_f64(g.exposure, 0.0, -3.0, 3.0);
    let brightness = sanitize_f64(g.brightness, 0.0, -0.5, 0.5);
    let contrast = sanitize_f64(g.contrast, 1.0, 0.4, 2.0);
    let sat = sanitize_f64(g.saturation, 1.0, 0.0, 3.0);
    let temp = sanitize_f64(g.temp, 0.0, -5.0, 5.0);
    let tint = sanitize_f64(g.tint, 0.0, -5.0, 5.0);
    let rs = sanitize_f64(g.lift.r, 0.0, -1.0, 1.0);
    let gs = sanitize_f64(g.lift.g, 0.0, -1.0, 1.0);
    let bs = sanitize_f64(g.lift.b, 0.0, -1.0, 1.0);
    let rm = (sanitize_f64(g.gamma.r, 0.0, -1.0, 1.0) + temp * 0.16).clamp(-1.0, 1.0);
    let gm = (sanitize_f64(g.gamma.g, 0.0, -1.0, 1.0) + tint * 0.14).clamp(-1.0, 1.0);
    let bm = (sanitize_f64(g.gamma.b, 0.0, -1.0, 1.0) - temp * 0.16).clamp(-1.0, 1.0);
    let rh = sanitize_f64(g.gain.r, 0.0, -1.0, 1.0);
    let gh = sanitize_f64(g.gain.g, 0.0, -1.0, 1.0);
    let bh = sanitize_f64(g.gain.b, 0.0, -1.0, 1.0);

    let enable_suffix = enable_expr
        .map(|expr| format!(":enable='{expr}'"))
        .unwrap_or_default();

    let cb = format!(
        "colorbalance=rs={rs:.3}:gs={gs:.3}:bs={bs:.3}:rm={rm:.3}:gm={gm:.3}:bm={bm:.3}:rh={rh:.3}:gh={gh:.3}:bh={bh:.3}{enable_suffix}"
    );
    let eq_base = format!(
        "eq=contrast={contrast:.3}:saturation={sat:.3}:brightness={brightness:.3}{enable_suffix}"
    );
    let eq = if exposure.abs() > 0.001 {
        format!("exposure=exposure={exposure:.3}{enable_suffix},{eq_base}")
    } else {
        eq_base
    };
    (cb, eq)
}

/// Shared color-grade + LUT intensity blend filter builder used by both export and frame capture.
#[allow(clippy::too_many_arguments)]
pub fn append_color_grade_filters(
    filters: &mut Vec<String>,
    input_tag: &str,
    output_tag: &str,
    label_prefix: &str,
    g: &ColorGrade,
    resolved_lut_path: Option<&Path>,
    preserve_alpha: bool,
    enable_expr: Option<&str>,
) {
    let (cb, eq) = build_color_grade_core_filters(g, enable_expr);
    let grade_core = format!("{cb},{eq}");
    let enable_suffix = enable_expr
        .map(|expr| format!(":enable='{expr}'"))
        .unwrap_or_default();
    let lut_intensity = sanitize_f64(g.lut_intensity, 0.85, 0.0, 1.0);

    let build_rgb_chain = |in_lbl: &str, out_lbl: &str| -> String {
        if let Some(lut_path) = resolved_lut_path
            && lut_intensity > 0.001
        {
            let escaped = escape_ffmpeg_filter_path(&lut_path.to_string_lossy());
            if lut_intensity >= 0.999 {
                format!("{in_lbl}{grade_core},lut3d=file={escaped}{enable_suffix}{out_lbl}")
            } else {
                let inv = 1.0 - lut_intensity;
                let lo = format!("[{label_prefix}_lut_orig]");
                let li = format!("[{label_prefix}_lut_in]");
                let lout = format!("[{label_prefix}_lut_out]");
                format!(
                    "{in_lbl}{grade_core},split=2{lo}{li};{li}lut3d=file={escaped}{enable_suffix}{lout};{lo}{lout}blend=all_expr='A*{inv:.3}+B*{lut_intensity:.3}'{enable_suffix}{out_lbl}"
                )
            }
        } else {
            format!("{in_lbl}{grade_core}{out_lbl}")
        }
    };

    if preserve_alpha {
        let rgb_in = format!("[{label_prefix}_rgb_in]");
        let a_in = format!("[{label_prefix}_a_in]");
        let a_ext = format!("[{label_prefix}_alpha]");
        let rgb_out = format!("[{label_prefix}_rgb_out]");
        let rgb_rgba = format!("[{label_prefix}_rgba]");
        let graded_rgb = build_rgb_chain(&rgb_in, &rgb_out);
        filters.push(format!(
            "{input_tag}split=2{rgb_in}{a_in};{a_in}alphaextract{a_ext};{graded_rgb};{rgb_out}format=rgba{rgb_rgba};{rgb_rgba}{a_ext}alphamerge{output_tag}"
        ));
    } else {
        filters.push(build_rgb_chain(input_tag, output_tag));
    }
}

fn ensure_even(n: u32) -> u32 {
    if n <= 2 { 2 } else { n & !1 }
}

fn has_disallowed_scheme(trimmed: &str) -> bool {
    let bytes = trimmed.as_bytes();
    // Allow Windows drive letters like `C:\...` or `C:/...`
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return false;
    }
    if let Some(colon_idx) = trimmed.find(':') {
        let prefix = &trimmed[..colon_idx];
        if !prefix.is_empty()
            && prefix.as_bytes()[0].is_ascii_alphabetic()
            && prefix
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-'))
        {
            return true;
        }
    }
    false
}

pub(crate) fn validate_local_media_path(raw: &str) -> Result<(), MediaError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(MediaError::StorageError("media path is empty".to_string()));
    }
    if trimmed.starts_with('-') {
        return Err(MediaError::StorageError(format!(
            "media path must not start with '-': {trimmed}"
        )));
    }
    if trimmed.contains("://") || has_disallowed_scheme(trimmed) {
        return Err(MediaError::StorageError(format!(
            "URL or FFmpeg protocol scheme is not allowed in local media path: {trimmed}"
        )));
    }
    Ok(())
}

/// FFmpeg unquoted 2-level filtergraph path escaping (for `lut3d=file=...`, `whisper=model=...`).
pub fn escape_ffmpeg_filter_path(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 12);
    for ch in raw.chars() {
        match ch {
            '\\' => out.push_str("\\\\\\\\"),
            '\'' => out.push_str("\\\\\\'"),
            ':' => out.push_str("\\\\:"),
            ',' => out.push_str("\\,"),
            ';' => out.push_str("\\;"),
            '[' => out.push_str("\\["),
            ']' => out.push_str("\\]"),
            _ => out.push(ch),
        }
    }
    out
}

fn build_audio_clip_chain(
    input_idx: usize,
    clip: &Clip,
    out_tag: &str,
    pre_trimmed: bool,
) -> String {
    let start_sec = if clip.start_sec.is_finite() {
        clip.start_sec.max(0.0)
    } else {
        0.0
    };
    let duration_sec = if clip.duration_sec.is_finite() {
        clip.duration_sec.max(0.0)
    } else {
        0.0
    };
    let delay_ms = (start_sec * 1000.0).round() as u64;
    let gain_db = sanitize_f64(clip.audio.gain_db, 0.0, -120.0, 36.0);
    let mut chain = if pre_trimmed {
        format!(
            "[{input_idx}:a]atrim=duration={duration_sec:.3},asetpts=PTS-STARTPTS,volume={gain_db:.2}dB"
        )
    } else {
        let src_in = if clip.source_in_sec.is_finite() {
            clip.source_in_sec.max(0.0)
        } else {
            0.0
        };
        format!(
            "[{input_idx}:a]atrim=start={src_in:.3}:duration={duration_sec:.3},asetpts=PTS-STARTPTS,volume={gain_db:.2}dB"
        )
    };
    let fade_in_raw = sanitize_f64(clip.audio.fade_in_sec, 0.0, 0.0, duration_sec);
    if fade_in_raw > 0.001 && duration_sec > 0.0 {
        let fade_in = fade_in_raw.min(duration_sec);
        chain.push_str(&format!(",afade=t=in:st=0:d={fade_in:.3}"));
    }
    let fade_out_raw = if clip.audio.fade_out_sec.is_finite() {
        clip.audio.fade_out_sec.max(0.0)
    } else {
        0.0
    };
    if fade_out_raw > 0.001 && duration_sec > 0.0 {
        let fade_out = fade_out_raw.min(duration_sec);
        let fade_start = (duration_sec - fade_out).max(0.0);
        chain.push_str(&format!(",afade=t=out:st={fade_start:.3}:d={fade_out:.3}"));
    }
    chain.push_str(&format!(",adelay={delay_ms}:all=1{out_tag}"));
    chain
}

fn media_path_for_audio<'a>(
    clip: &Clip,
    media_map: &HashMap<&str, &'a MediaAsset>,
) -> Option<&'a str> {
    if let ClipKind::Media { media_id, .. } = &clip.payload
        && let Some(asset) = media_map.get(media_id.as_str())
    {
        match asset.kind {
            MediaKind::Audio => return Some(asset.path.as_str()),
            MediaKind::Video => {
                if asset.intelligence.has_audio
                    || !asset.intelligence.waveform_peaks.is_empty()
                    || asset.intelligence.integrated_lufs.is_some()
                {
                    return Some(asset.path.as_str());
                }
            }
            MediaKind::Image | MediaKind::Document => {}
        }
    }
    None
}

fn codec_args_for(
    format: ExportFormat,
    quality: ExportQuality,
    hw_encoders: &[String],
) -> Vec<String> {
    match format {
        ExportFormat::Mp4 => {
            let use_vt = hw_encoders.iter().any(|e| e == "h264_videotoolbox");
            let use_nvenc = !use_vt && hw_encoders.iter().any(|e| e == "h264_nvenc");
            if use_vt {
                let bitrate = match quality {
                    ExportQuality::Draft => "3500k",
                    ExportQuality::Standard => "8000k",
                    ExportQuality::High => "16000k",
                };
                vec![
                    "-c:v".into(),
                    "h264_videotoolbox".into(),
                    "-b:v".into(),
                    bitrate.into(),
                    "-pix_fmt".into(),
                    "yuv420p".into(),
                    "-c:a".into(),
                    "aac".into(),
                    "-ar".into(),
                    "48000".into(),
                    "-b:a".into(),
                    "192k".into(),
                    "-movflags".into(),
                    "+faststart".into(),
                ]
            } else if use_nvenc {
                let (preset, cq, bitrate) = match quality {
                    ExportQuality::Draft => ("p2", "28", "4000k"),
                    ExportQuality::Standard => ("p4", "22", "8000k"),
                    ExportQuality::High => ("p6", "18", "16000k"),
                };
                vec![
                    "-c:v".into(),
                    "h264_nvenc".into(),
                    "-preset".into(),
                    preset.into(),
                    "-cq".into(),
                    cq.into(),
                    "-b:v".into(),
                    bitrate.into(),
                    "-pix_fmt".into(),
                    "yuv420p".into(),
                    "-c:a".into(),
                    "aac".into(),
                    "-ar".into(),
                    "48000".into(),
                    "-b:a".into(),
                    "192k".into(),
                    "-movflags".into(),
                    "+faststart".into(),
                ]
            } else {
                let (preset, crf) = match quality {
                    ExportQuality::Draft => ("veryfast", "26"),
                    ExportQuality::Standard => ("medium", "20"),
                    ExportQuality::High => ("slow", "16"),
                };
                vec![
                    "-c:v".into(),
                    "libx264".into(),
                    "-preset".into(),
                    preset.into(),
                    "-crf".into(),
                    crf.into(),
                    "-pix_fmt".into(),
                    "yuv420p".into(),
                    "-c:a".into(),
                    "aac".into(),
                    "-ar".into(),
                    "48000".into(),
                    "-b:a".into(),
                    "192k".into(),
                    "-movflags".into(),
                    "+faststart".into(),
                ]
            }
        }
        ExportFormat::MovProRes4444 => vec![
            "-c:v".into(),
            "prores_ks".into(),
            "-profile:v".into(),
            "4444".into(),
            "-pix_fmt".into(),
            "yuva444p10le".into(),
            "-c:a".into(),
            "aac".into(),
            "-ar".into(),
            "48000".into(),
            "-b:a".into(),
            "256k".into(),
        ],
        ExportFormat::WebmAlpha => {
            let (crf, cpu_used) = match quality {
                ExportQuality::Draft => ("36", "4"),
                ExportQuality::Standard => ("28", "2"),
                ExportQuality::High => ("20", "1"),
            };
            vec![
                "-c:v".into(),
                "libvpx-vp9".into(),
                "-b:v".into(),
                "0".into(),
                "-crf".into(),
                crf.into(),
                "-deadline".into(),
                "good".into(),
                "-cpu-used".into(),
                cpu_used.into(),
                "-pix_fmt".into(),
                "yuva420p".into(),
                "-auto-alt-ref".into(),
                "0".into(),
                "-c:a".into(),
                "libopus".into(),
                "-ar".into(),
                "48000".into(),
                "-b:a".into(),
                "192k".into(),
            ]
        }
    }
}

pub fn execute_export(
    toolchain: &MediaToolchain,
    project: &Project,
    req: &ExportRequest,
    output_path: &Path,
    overlay_sequence_dir: Option<&Path>,
) -> Result<PathBuf, MediaError> {
    let caps = toolchain.detect_capabilities();
    execute_export_with_progress(
        toolchain,
        project,
        req,
        output_path,
        ExportPlanOptions {
            overlay_sequence_dir,
            overlay_images: &[],
            resolved_lut_path: project.active_grade.lut_path.as_deref().map(Path::new),
        },
        &caps.hw_encoders,
        |_| {},
    )
}

fn run_ffmpeg_plan<F>(
    ffmpeg: &Path,
    plan: &ExportFfmpegPlan,
    total_us: f64,
    on_progress: &mut F,
) -> Result<(), MediaError>
where
    F: FnMut(f32),
{
    const INLINE_FILTER_COMPLEX_MAX_BYTES: usize = 24_000;
    if plan.filter_complex.len() <= INLINE_FILTER_COMPLEX_MAX_BYTES {
        return run_ffmpeg_plan_inner(ffmpeg, plan, None, false, total_us, on_progress);
    }

    let script_path = std::env::temp_dir().join(format!("solaris_fc_{}.txt", Uuid::new_v4()));
    std::fs::write(&script_path, &plan.filter_complex)?;
    let _script_guard = TempFileGuard(script_path.clone());

    match run_ffmpeg_plan_inner(
        ffmpeg,
        plan,
        Some(&script_path),
        false,
        total_us,
        on_progress,
    ) {
        Ok(()) => Ok(()),
        Err(MediaError::ProcessFailed(msg))
            if msg.contains("Unrecognized option '/filter_complex'")
                || msg.contains("Option not found") =>
        {
            run_ffmpeg_plan_inner(
                ffmpeg,
                plan,
                Some(&script_path),
                true,
                total_us,
                on_progress,
            )
        }
        Err(err) => Err(err),
    }
}

fn run_ffmpeg_plan_inner<F>(
    ffmpeg: &Path,
    plan: &ExportFfmpegPlan,
    script_path: Option<&Path>,
    legacy_script_flag: bool,
    total_us: f64,
    on_progress: &mut F,
) -> Result<(), MediaError>
where
    F: FnMut(f32),
{
    let mut cmd = Command::new(ffmpeg);
    cmd.args([
        "-y",
        "-hide_banner",
        "-v",
        "error",
        "-nostats",
        "-progress",
        "pipe:1",
    ]);
    cmd.args(&plan.inputs);
    match script_path {
        Some(path) if legacy_script_flag => {
            cmd.arg("-filter_complex_script").arg(path);
        }
        Some(path) => {
            cmd.arg("-/filter_complex").arg(path);
        }
        None => {
            cmd.arg("-filter_complex").arg(&plan.filter_complex);
        }
    }
    cmd.args(["-map", &plan.video_map]);
    if let Some(amap) = &plan.audio_map {
        cmd.args(["-map", amap]);
    }
    cmd.args(&plan.codec_args);
    cmd.arg(&plan.output_path);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn()?;
    // Drain stderr concurrently on a background thread to eliminate OS pipe deadlocks
    let stderr_handle = child.stderr.take().map(|mut err_pipe| {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = err_pipe.read_to_string(&mut buf);
            buf
        })
    });

    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if let Some(val) = line.strip_prefix("out_time_us=")
                && let Ok(us) = val.trim().parse::<f64>()
            {
                let frac = (us / total_us).clamp(0.0, 0.99) as f32;
                on_progress(frac);
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
    Ok(())
}

pub fn execute_export_with_progress<F>(
    toolchain: &MediaToolchain,
    project: &Project,
    req: &ExportRequest,
    output_path: &Path,
    options: ExportPlanOptions<'_>,
    hw_encoders: &[String],
    mut on_progress: F,
) -> Result<PathBuf, MediaError>
where
    F: FnMut(f32),
{
    let ffmpeg = toolchain
        .ffmpeg_path
        .as_ref()
        .ok_or_else(|| MediaError::ToolchainUnavailable("ffmpeg not found".to_string()))?;

    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }

    let total_us = (project.duration_sec().max(1.0) * 1_000_000.0).max(1.0);
    let plan =
        build_export_plan_with_options(project, req, output_path, options.clone(), hw_encoders)?;
    let used_hw = req.format == ExportFormat::Mp4
        && (plan.codec_args.iter().any(|a| a == "h264_videotoolbox")
            || plan.codec_args.iter().any(|a| a == "h264_nvenc"));

    match run_ffmpeg_plan(ffmpeg, &plan, total_us, &mut on_progress) {
        Ok(()) => Ok(output_path.to_path_buf()),
        Err(hw_err) if used_hw => {
            tracing::warn!(
                "hardware video export failed ({hw_err}); falling back to libx264 software encoder"
            );
            let sw_plan = build_export_plan_with_options(project, req, output_path, options, &[])?;
            run_ffmpeg_plan(ffmpeg, &sw_plan, total_us, &mut on_progress)?;
            Ok(output_path.to_path_buf())
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use solaris_core_ir::{
        AspectPreset, AudioSettings, MediaAsset, MediaIntelligenceManifest, SmartReframeBox,
        Transform2D,
    };

    fn seeded_project() -> Project {
        let mut p = Project::new_default("export_test", AspectPreset::Vertical9x16);
        p.media_pool.push(MediaAsset {
            id: "m_aroll".into(),
            name: "aroll.mp4".into(),
            kind: MediaKind::Video,
            role_hint: "a_roll".into(),
            path: "/tmp/aroll.mp4".into(),
            added_at: Utc::now(),
            intelligence: MediaIntelligenceManifest::default(),
        });
        p.media_pool.push(MediaAsset {
            id: "m_voice".into(),
            name: "voice.wav".into(),
            kind: MediaKind::Audio,
            role_hint: "voice".into(),
            path: "/tmp/voice.wav".into(),
            added_at: Utc::now(),
            intelligence: MediaIntelligenceManifest::default(),
        });
        p.media_pool.push(MediaAsset {
            id: "m_music".into(),
            name: "beat.wav".into(),
            kind: MediaKind::Audio,
            role_hint: "music".into(),
            path: "/tmp/beat.wav".into(),
            added_at: Utc::now(),
            intelligence: MediaIntelligenceManifest::default(),
        });

        p.tracks[3].clips.push(Clip {
            id: "c_v1".into(),
            label: "A-Roll".into(),
            start_sec: 0.0,
            duration_sec: 4.0,
            source_in_sec: 0.0,
            transform: Transform2D::default(),
            audio: AudioSettings::default(),
            payload: ClipKind::Media {
                media_id: "m_aroll".into(),
                ken_burns: false,
            },
        });
        p.tracks[5].clips.push(Clip {
            id: "c_a1".into(),
            label: "Voice".into(),
            start_sec: 0.0,
            duration_sec: 4.0,
            source_in_sec: 0.0,
            transform: Transform2D::default(),
            audio: AudioSettings::default(),
            payload: ClipKind::Media {
                media_id: "m_voice".into(),
                ken_burns: false,
            },
        });
        p.tracks[6].clips.push(Clip {
            id: "c_a2".into(),
            label: "Beat".into(),
            start_sec: 0.0,
            duration_sec: 4.0,
            source_in_sec: 0.0,
            transform: Transform2D::default(),
            audio: AudioSettings {
                gain_db: -14.0,
                ducking_enabled: true,
                duck_target_db: -18.0,
                fade_in_sec: 0.25,
                fade_out_sec: 0.5,
            },
            payload: ClipKind::Media {
                media_id: "m_music".into(),
                ken_burns: false,
            },
        });
        p
    }

    #[test]
    fn builds_filtergraph_with_exact_input_indices_grade_ducking_apad_and_lufs() {
        let p = seeded_project();
        let req = ExportRequest {
            format: ExportFormat::Mp4,
            quality: ExportQuality::Standard,
            target_lufs: -14.0,
            output_name: None,
        };
        let plan = build_export_plan(&p, &req, Path::new("/tmp/out.mp4"), None, &[]).unwrap();
        assert!(plan.filter_complex.contains("[0:v]"));
        assert!(plan.filter_complex.contains("[1:a]"));
        assert!(plan.filter_complex.contains("[2:a]"));
        assert!(!plan.filter_complex.contains("[4:a]"));
        assert!(plan.filter_complex.contains("colorbalance="));
        assert!(plan.filter_complex.contains("apad=whole_dur=4.000"));
        assert!(plan.filter_complex.contains("sidechaincompress="));
        assert!(
            plan.filter_complex
                .contains("loudnorm=I=-14.0:TP=-1.5:LRA=11,aresample=48000")
        );
        assert!(plan.codec_args.windows(2).any(|w| w == ["-ar", "48000"]));
        assert!(plan.filter_complex.contains("adelay=0:all=1"));
        assert!(plan.filter_complex.contains("afade=t=in"));
        assert!(plan.filter_complex.contains("afade=t=out"));
        assert_eq!(plan.audio_map.as_deref(), Some("[afinal]"));
    }

    #[test]
    fn builds_alpha_overlay_ken_burns_lut_intensity_and_small_scale_pad() {
        let mut p = seeded_project();
        p.active_grade.lut_intensity = 0.5;
        p.media_pool.push(MediaAsset {
            id: "m_img".into(),
            name: "still.png".into(),
            kind: MediaKind::Image,
            role_hint: "b_roll".into(),
            path: "/tmp/still.png".into(),
            added_at: Utc::now(),
            intelligence: MediaIntelligenceManifest::default(),
        });
        p.tracks[2].clips.push(Clip {
            id: "c_v2_img".into(),
            label: "Still Ken Burns".into(),
            start_sec: 0.5,
            duration_sec: 2.0,
            source_in_sec: 0.0,
            transform: Transform2D {
                scale: 0.5,
                rotation_deg: 5.0,
                smart_reframe: Some(SmartReframeBox {
                    x: 0.25,
                    y: 0.0,
                    width: 0.5,
                    height: 1.0,
                }),
                ..Transform2D::default()
            },
            audio: AudioSettings::default(),
            payload: ClipKind::Media {
                media_id: "m_img".into(),
                ken_burns: true,
            },
        });
        p.tracks[4].clips.push(Clip {
            id: "c_fx_1".into(),
            label: "Warm Boost".into(),
            start_sec: 1.0,
            duration_sec: 2.0,
            source_in_sec: 0.0,
            transform: Transform2D::default(),
            audio: AudioSettings::default(),
            payload: ClipKind::Grade {
                grade: ColorGrade {
                    contrast: 1.15,
                    ..ColorGrade::default()
                },
            },
        });

        let req = ExportRequest {
            format: ExportFormat::MovProRes4444,
            quality: ExportQuality::High,
            target_lufs: -14.0,
            output_name: None,
        };
        let overlays = [OverlayImageSpec::single(
            PathBuf::from("/tmp/ov1.png"),
            0.5,
            2.5,
        )];
        let plan = build_export_plan_with_options(
            &p,
            &req,
            Path::new("/tmp/out.mov"),
            ExportPlanOptions {
                overlay_sequence_dir: None,
                overlay_images: &overlays,
                resolved_lut_path: Some(Path::new("/tmp/o'reilly/warm:look,1[a].cube")),
            },
            &[],
        )
        .unwrap();

        assert!(plan.filter_complex.contains("color=c=black@0.0"));
        assert!(!plan.inputs.contains(&"-loop".to_string()));
        assert!(plan.filter_complex.contains("format=rgba,pad=1080:1920"));
        assert!(
            plan.filter_complex
                .contains("scale=2160:3840:flags=lanczos")
        );
        assert!(plan.filter_complex.contains("min(zoom+0.002000,1.12)"));
        assert!(plan.filter_complex.contains("x='iw/2-(iw/zoom/2)'"));
        assert!(plan.filter_complex.contains("crop=iw*0.5000:ih*1.0000"));
        assert!(plan.filter_complex.contains("rotate="));
        assert!(plan.filter_complex.contains("alphaextract"));
        assert!(plan.filter_complex.contains("alphamerge"));
        assert!(plan.filter_complex.contains(":format=rgb"));
        assert!(plan.filter_complex.contains("blend=all_expr="));
        assert!(
            plan.filter_complex
                .contains("lut3d=file=/tmp/o\\\\\\'reilly/warm\\\\:look\\,1\\[a\\].cube")
        );
        assert!(!plan.filter_complex.contains("lut3d=file='"));
        assert!(plan.filter_complex.contains("eof_action=repeat"));
        assert!(plan.filter_complex.contains("between(t,0.500,2.500)"));
        // FX adjustment layer uses inline enable instead of split+overlay (preventing alpha doubling)
        assert!(plan.filter_complex.contains("vfx_done_0"));
        assert!(
            plan.filter_complex
                .contains(":enable='between(t,1.000,3.000)'")
        );
        assert!(!plan.filter_complex.contains("vfx_base_0"));
        assert!(plan.codec_args.contains(&"prores_ks".to_string()));
        assert!(plan.codec_args.contains(&"yuva444p10le".to_string()));
        assert!(plan.codec_args.windows(2).any(|w| w == ["-ar", "48000"]));
    }

    #[test]
    fn partitions_ducked_music_and_unducked_sfx_and_respects_a1_mute() {
        let mut p = seeded_project();
        p.media_pool[0].intelligence.has_audio = true;
        p.media_pool.push(MediaAsset {
            id: "m_sfx".into(),
            name: "whoosh.wav".into(),
            kind: MediaKind::Audio,
            role_hint: "sfx".into(),
            path: "/tmp/whoosh.wav".into(),
            added_at: Utc::now(),
            intelligence: MediaIntelligenceManifest::default(),
        });
        p.tracks[6].clips.push(Clip {
            id: "c_sfx".into(),
            label: "Whoosh SFX".into(),
            start_sec: 1.0,
            duration_sec: 0.4,
            source_in_sec: 0.0,
            transform: Transform2D::default(),
            audio: AudioSettings {
                gain_db: 0.0,
                ducking_enabled: false,
                duck_target_db: -18.0,
                fade_in_sec: 0.0,
                fade_out_sec: 0.8, // longer than duration_sec -> still emits clamped afade out
            },
            payload: ClipKind::Media {
                media_id: "m_sfx".into(),
                ken_burns: false,
            },
        });

        let req = ExportRequest {
            format: ExportFormat::Mp4,
            quality: ExportQuality::Standard,
            target_lufs: -14.0,
            output_name: None,
        };
        let plan = build_export_plan(&p, &req, Path::new("/tmp/out.mp4"), None, &[]).unwrap();
        // Music clip [a2] is ducked via sidechaincompress, while SFX [a3] joins final mix unducked
        assert!(plan.filter_complex.contains("[a2][vkey]sidechaincompress="));
        assert!(
            plan.filter_complex
                .contains("[vmain][mducked][a3]amix=inputs=3")
        );
        // Short clip with fade_out > duration still emits clamped fade out
        assert!(plan.filter_complex.contains("afade=t=out:st=0.000:d=0.400"));

        // Muting A1 must NOT fall back and unmute V1 embedded audio
        p.tracks[5].muted = true;
        let muted_plan = build_export_plan(&p, &req, Path::new("/tmp/out.mp4"), None, &[]).unwrap();
        assert!(!muted_plan.filter_complex.contains("[0:a]"));
        assert!(!muted_plan.filter_complex.contains("sidechaincompress="));
    }

    #[test]
    fn folds_pan_into_crop_for_zoomed_clips_and_rejects_bad_paths() {
        let mut p = seeded_project();
        p.tracks[3].clips[0].transform = Transform2D {
            x: 0.1,
            y: -0.05,
            scale: 1.2,
            ..Transform2D::default()
        };
        let req = ExportRequest {
            format: ExportFormat::Mp4,
            quality: ExportQuality::Standard,
            target_lufs: -14.0,
            output_name: None,
        };
        let plan = build_export_plan(&p, &req, Path::new("/tmp/out.mp4"), None, &[]).unwrap();
        assert!(plan.filter_complex.contains(
            "crop=1080:1920:clamp((iw-1080)/2-(108),0,iw-1080):clamp((ih-1920)/2-(-96),0,ih-1920)"
        ));

        // Leading dash and arbitrary protocol schemes are rejected
        assert!(validate_local_media_path("-i").is_err());
        assert!(validate_local_media_path("lavfi:testsrc").is_err());
        assert!(validate_local_media_path("custom+proto:foo").is_err());
        assert!(validate_local_media_path(r"C:\Users\solaris\clip.mp4").is_ok());
        assert!(validate_local_media_path("D:/media/clip.mp4").is_ok());
        assert!(validate_local_media_path("/tmp/o'reilly/warm:look.cube").is_ok());

        // resolved_lut_path is also validated
        assert!(
            build_export_plan_with_options(
                &p,
                &req,
                Path::new("/tmp/out.mp4"),
                ExportPlanOptions {
                    overlay_sequence_dir: None,
                    overlay_images: &[],
                    resolved_lut_path: Some(Path::new("http://169.254.169.254/evil.cube")),
                },
                &[],
            )
            .is_err()
        );
    }

    #[test]
    fn webm_alpha_uses_crf_nvenc_wired_and_rejects_ssrf_urls() {
        let mut p = seeded_project();
        let webm_req = ExportRequest {
            format: ExportFormat::WebmAlpha,
            quality: ExportQuality::High,
            target_lufs: f64::NAN,
            output_name: None,
        };
        let webm = build_export_plan(&p, &webm_req, Path::new("/tmp/out.webm"), None, &[]).unwrap();
        assert!(webm.codec_args.contains(&"-crf".to_string()));
        assert!(webm.codec_args.contains(&"20".to_string()));
        assert!(webm.codec_args.windows(2).any(|w| w == ["-ar", "48000"]));
        assert!(webm.filter_complex.contains("loudnorm=I=-14.0"));

        let nv_req = ExportRequest {
            format: ExportFormat::Mp4,
            quality: ExportQuality::Standard,
            target_lufs: -14.0,
            output_name: None,
        };
        let nv = build_export_plan(
            &p,
            &nv_req,
            Path::new("/tmp/out.mp4"),
            None,
            &["h264_nvenc".to_string()],
        )
        .unwrap();
        assert!(nv.codec_args.contains(&"h264_nvenc".to_string()));

        p.media_pool[0].path = "http://169.254.169.254/latest/meta-data".into();
        assert!(build_export_plan(&p, &nv_req, Path::new("/tmp/out.mp4"), None, &[]).is_err());
    }

    #[test]
    fn honors_track_solo_and_full_clip_fade_out() {
        let mut p = seeded_project();
        p.tracks[6].solo = true;
        p.tracks[6].clips[0].audio.fade_out_sec = 4.0; // equals duration_sec
        let req = ExportRequest {
            format: ExportFormat::Mp4,
            quality: ExportQuality::Standard,
            target_lufs: -14.0,
            output_name: None,
        };
        let plan = build_export_plan(&p, &req, Path::new("/tmp/out.mp4"), None, &[]).unwrap();
        // Voice track was not soloed, so only music should be mixed (no sidechaincompress)
        assert!(!plan.filter_complex.contains("sidechaincompress="));
        assert!(plan.filter_complex.contains("afade=t=out:st=0.000:d=4.000"));
    }

    #[test]
    fn executes_real_ffmpeg_when_available() {
        let toolchain = MediaToolchain::from_env();
        let Some(ffmpeg) = &toolchain.ffmpeg_path else {
            return;
        };

        let dir = std::env::temp_dir().join(format!("solaris_export_e2e_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let src_vid = dir.join("src.mp4");
        let out_vid = dir.join("out.mp4");

        let gen_out = Command::new(ffmpeg)
            .args([
                "-y",
                "-hide_banner",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=320x240:rate=30:duration=1",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=1",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-shortest",
            ])
            .arg(&src_vid)
            .output()
            .unwrap();
        assert!(gen_out.status.success(), "failed generating test video");

        let mut p = Project::new_default("e2e_test", AspectPreset::Vertical9x16);
        p.canvas.width = 320;
        p.canvas.height = 480;
        p.media_pool.push(MediaAsset {
            id: "m1".into(),
            name: "src.mp4".into(),
            kind: MediaKind::Video,
            role_hint: "a_roll".into(),
            path: src_vid.to_string_lossy().to_string(),
            added_at: Utc::now(),
            intelligence: MediaIntelligenceManifest {
                analyzed: true,
                duration_sec: 1.0,
                has_audio: true,
                ..MediaIntelligenceManifest::default()
            },
        });
        p.tracks[3].clips.push(Clip {
            id: "c1".into(),
            label: "Src".into(),
            start_sec: 0.0,
            duration_sec: 1.0,
            source_in_sec: 0.0,
            transform: Transform2D::default(),
            audio: AudioSettings::default(),
            payload: ClipKind::Media {
                media_id: "m1".into(),
                ken_burns: false,
            },
        });

        let req = ExportRequest {
            format: ExportFormat::Mp4,
            quality: ExportQuality::Draft,
            target_lufs: -14.0,
            output_name: None,
        };
        let rendered = execute_export(&toolchain, &p, &req, &out_vid, None).unwrap();
        assert!(rendered.is_file());
        assert!(std::fs::metadata(&rendered).unwrap().len() > 1000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn preserves_v1_audio_outside_partial_a1_and_mixes_unmuted_v2_audio() {
        let mut p = Project::new_default("v1_a1_v2_audio", AspectPreset::Vertical9x16);
        p.media_pool.push(MediaAsset {
            id: "m_v1".into(),
            name: "v1.mp4".into(),
            kind: MediaKind::Video,
            role_hint: "a_roll".into(),
            path: "/tmp/v1.mp4".into(),
            added_at: Utc::now(),
            intelligence: MediaIntelligenceManifest {
                has_audio: true,
                duration_sec: 10.0,
                ..MediaIntelligenceManifest::default()
            },
        });
        p.media_pool.push(MediaAsset {
            id: "m_v2_sound".into(),
            name: "v2_sound.mp4".into(),
            kind: MediaKind::Video,
            role_hint: "b_roll".into(),
            path: "/tmp/v2_sound.mp4".into(),
            added_at: Utc::now(),
            intelligence: MediaIntelligenceManifest {
                has_audio: true,
                duration_sec: 3.0,
                ..MediaIntelligenceManifest::default()
            },
        });
        p.media_pool.push(MediaAsset {
            id: "m_v2_muted".into(),
            name: "v2_muted.mp4".into(),
            kind: MediaKind::Video,
            role_hint: "b_roll".into(),
            path: "/tmp/v2_muted.mp4".into(),
            added_at: Utc::now(),
            intelligence: MediaIntelligenceManifest {
                has_audio: true,
                duration_sec: 2.0,
                ..MediaIntelligenceManifest::default()
            },
        });
        p.media_pool.push(MediaAsset {
            id: "m_vo".into(),
            name: "vo.wav".into(),
            kind: MediaKind::Audio,
            role_hint: "voice".into(),
            path: "/tmp/vo.wav".into(),
            added_at: Utc::now(),
            intelligence: MediaIntelligenceManifest::default(),
        });

        // Two V1 clips: 0..4s (no A1 overlap) and 4..8s (overlapped by A1 4..8s)
        p.tracks[3].clips.push(Clip {
            id: "v1_1".into(),
            label: "V1 1".into(),
            start_sec: 0.0,
            duration_sec: 4.0,
            source_in_sec: 0.0,
            transform: Transform2D::default(),
            audio: AudioSettings::default(),
            payload: ClipKind::Media {
                media_id: "m_v1".into(),
                ken_burns: false,
            },
        });
        p.tracks[3].clips.push(Clip {
            id: "v1_2".into(),
            label: "V1 2".into(),
            start_sec: 4.0,
            duration_sec: 4.0,
            source_in_sec: 4.0,
            transform: Transform2D::default(),
            audio: AudioSettings::default(),
            payload: ClipKind::Media {
                media_id: "m_v1".into(),
                ken_burns: false,
            },
        });
        // Two V2 clips: one unmuted (gain 0), one muted (gain -99)
        p.tracks[2].clips.push(Clip {
            id: "v2_1".into(),
            label: "V2 Sound".into(),
            start_sec: 1.0,
            duration_sec: 2.0,
            source_in_sec: 0.0,
            transform: Transform2D::default(),
            audio: AudioSettings::default(),
            payload: ClipKind::Media {
                media_id: "m_v2_sound".into(),
                ken_burns: false,
            },
        });
        p.tracks[2].clips.push(Clip {
            id: "v2_2".into(),
            label: "V2 Muted".into(),
            start_sec: 5.0,
            duration_sec: 2.0,
            source_in_sec: 0.0,
            transform: Transform2D::default(),
            audio: AudioSettings {
                gain_db: -99.0,
                ..AudioSettings::default()
            },
            payload: ClipKind::Media {
                media_id: "m_v2_muted".into(),
                ken_burns: false,
            },
        });
        // A1 voiceover covering only 4..8s
        p.tracks[5].clips.push(Clip {
            id: "a1_1".into(),
            label: "VO".into(),
            start_sec: 4.0,
            duration_sec: 4.0,
            source_in_sec: 0.0,
            transform: Transform2D::default(),
            audio: AudioSettings::default(),
            payload: ClipKind::Media {
                media_id: "m_vo".into(),
                ken_burns: false,
            },
        });

        let req = ExportRequest {
            format: ExportFormat::Mp4,
            quality: ExportQuality::Standard,
            target_lufs: -14.0,
            output_name: None,
        };
        let plan = build_export_plan(&p, &req, Path::new("/tmp/out.mp4"), None, &[]).unwrap();
        // Input 0 (v1_1) kept, input 1 (v1_2) skipped due to A1 overlap, input 2 (v2_1) kept, input 3 (v2_2) muted, input 4 (a1_1) kept
        assert!(plan.filter_complex.contains("[0:a]"));
        assert!(!plan.filter_complex.contains("[1:a]"));
        assert!(plan.filter_complex.contains("[2:a]"));
        assert!(!plan.filter_complex.contains("[3:a]"));
        assert!(plan.filter_complex.contains("[4:a]"));
    }
}
