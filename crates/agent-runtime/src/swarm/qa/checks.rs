use std::sync::atomic::AtomicU32;

use solaris_core_ir::{
    AspectPreset, Clip, ClipKind, ColorGrade, Project, QaCheckItem, QaSeverity, SavedCodeAsset,
    TimelineCommand, TrackKind,
};
use tokio_util::sync::CancellationToken;

use crate::backend::SolarisBackend;
use crate::lint::lint_gfx_html;

use super::apply_locked_op;

fn is_interval_covered_by_track(project: &Project, track_id: &str, start: f64, end: f64) -> bool {
    let Some(track) = project.tracks.iter().find(|t| t.id == track_id && !t.muted) else {
        return false;
    };
    let mut intervals: Vec<(f64, f64)> = track
        .clips
        .iter()
        .map(|c| (c.start_sec, c.start_sec + c.duration_sec))
        .filter(|(s, e)| *e > start && *s < end)
        .collect();
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut covered_until = start;
    for (s, e) in intervals {
        if s > covered_until + 0.05 {
            return false;
        }
        covered_until = covered_until.max(e);
        if covered_until >= end - 0.05 {
            return true;
        }
    }
    covered_until >= end - 0.05
}

pub(super) fn check_timeline_pacing(project: &Project, checks: &mut Vec<QaCheckItem>) {
    let earliest_video = project
        .tracks
        .iter()
        .filter(|t| matches!(t.kind, TrackKind::ARoll | TrackKind::BRoll) && !t.muted)
        .flat_map(|t| t.clips.iter())
        .map(|c| c.start_sec)
        .fold(f64::INFINITY, f64::min);

    if earliest_video.is_finite() && earliest_video > 0.5 {
        checks.push(QaCheckItem {
            id: "hook-lead-gap".to_string(),
            category: "timeline".to_string(),
            severity: QaSeverity::Warning,
            message: format!(
                "First video clip starts at {earliest_video:.2}s; hook should begin <= 0.50s."
            ),
            time_sec: Some(0.0),
            auto_fixed: false,
        });
    }

    if let Some(v1) = project.tracks.iter().find(|t| t.id == "v1" && !t.muted) {
        let mut sorted: Vec<&Clip> = v1.clips.iter().collect();
        sorted.sort_by(|a, b| a.start_sec.total_cmp(&b.start_sec));
        let mut max_end: Option<f64> = None;
        for clip in sorted {
            if let Some(prev_end) = max_end {
                let gap = clip.start_sec - prev_end;
                if gap > 0.4
                    && !is_interval_covered_by_track(project, "v2", prev_end, clip.start_sec)
                {
                    checks.push(QaCheckItem {
                        id: format!("v1-gap-{prev_end:.2}"),
                        category: "timeline".to_string(),
                        severity: QaSeverity::Warning,
                        message: format!(
                            "V1 has an uncovered {gap:.2}s dead gap at {prev_end:.2}s between clips."
                        ),
                        time_sec: Some(prev_end),
                        auto_fixed: false,
                    });
                    break;
                }
            }
            let clip_end = clip.start_sec + clip.duration_sec;
            max_end = Some(max_end.map_or(clip_end, |m| m.max(clip_end)));
        }
    }
}

fn is_neutral_grade(g: &ColorGrade) -> bool {
    (g.contrast - 1.0).abs() <= 0.01
        && (g.saturation - 1.0).abs() <= 0.01
        && g.exposure.abs() <= 0.01
        && g.temp.abs() <= 0.01
        && g.tint.abs() <= 0.01
        && g.lut_path.is_none()
}

pub(super) async fn check_and_fix_log_grade(
    backend: &dyn SolarisBackend,
    project: &mut Project,
    brand_lut: Option<&str>,
    auto_fix: bool,
    ops_counter: Option<&AtomicU32>,
    auto_fixes_applied: &mut u32,
    checks: &mut Vec<QaCheckItem>,
) -> bool {
    let has_placed_log = project
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .any(|clip| match &clip.payload {
            ClipKind::Media { media_id, .. } => project
                .media_pool
                .iter()
                .find(|m| m.id == *media_id)
                .is_some_and(|m| m.intelligence.is_log_footage),
            _ => false,
        });

    if !has_placed_log {
        return false;
    }

    let has_fx_grade_clip = project
        .tracks
        .iter()
        .find(|t| t.id == "fx" && !t.muted)
        .is_some_and(|fx| {
            fx.clips.iter().any(|c| match &c.payload {
                ClipKind::Grade { grade } => !is_neutral_grade(grade),
                _ => false,
            })
        });

    let g = &project.active_grade;
    let untouched = is_neutral_grade(g) && !has_fx_grade_clip;

    if !untouched {
        checks.push(QaCheckItem {
            id: "log-recovery".to_string(),
            category: "color".to_string(),
            severity: QaSeverity::Pass,
            message: "Log footage has an active Rec.709 recovery grade applied.".to_string(),
            time_sec: None,
            auto_fixed: false,
        });
        return false;
    }

    if !auto_fix {
        checks.push(QaCheckItem {
            id: "log-recovery".to_string(),
            category: "color".to_string(),
            severity: QaSeverity::Warning,
            message: "Log footage detected on timeline with neutral color grade.".to_string(),
            time_sec: None,
            auto_fixed: false,
        });
        return false;
    }

    let recovered = ColorGrade {
        contrast: 1.14,
        saturation: 1.12,
        temp: 0.04,
        lut_path: brand_lut.map(str::to_string),
        ..g.clone()
    };
    let fixed = apply_locked_op(
        backend,
        &project.id,
        "fx",
        TimelineCommand::SetColorGrade {
            grade: recovered,
            track_id: None,
            clip_id: None,
        },
        "Auto-recover Log footage to Rec.709",
        ops_counter,
    )
    .await;
    if fixed {
        *auto_fixes_applied += 1;
    }

    checks.push(QaCheckItem {
        id: "log-recovery".to_string(),
        category: "color".to_string(),
        severity: if fixed {
            QaSeverity::Fixed
        } else {
            QaSeverity::Warning
        },
        message: if fixed {
            "Applied Rec.709 Log recovery grade on FX.".to_string()
        } else {
            "Log footage detected with neutral grade (FX track locked).".to_string()
        },
        time_sec: None,
        auto_fixed: fixed,
    });
    fixed
}

pub(super) async fn check_and_fix_audio(
    backend: &dyn SolarisBackend,
    project: &Project,
    auto_fix: bool,
    cancel: &CancellationToken,
    ops_counter: Option<&AtomicU32>,
    auto_fixes_applied: &mut u32,
    checks: &mut Vec<QaCheckItem>,
) -> bool {
    let mut any_mutated = false;
    let mut speech_present = false;
    let mut lufs_checked = false;

    for track_id in ["a1", "v1"] {
        if cancel.is_cancelled() {
            return any_mutated;
        }
        let Some(track) = project.tracks.iter().find(|t| t.id == track_id) else {
            continue;
        };
        if track.muted {
            continue;
        }
        for clip in &track.clips {
            if cancel.is_cancelled() {
                return any_mutated;
            }
            // Skip intentionally muted/silent clips (e.g., B-roll / silent A-roll at -99 dB)
            if clip.audio.gain_db <= -90.0 {
                continue;
            }
            let ClipKind::Media { media_id, .. } = &clip.payload else {
                continue;
            };
            let Some(asset) = project.media_pool.iter().find(|m| m.id == *media_id) else {
                continue;
            };
            if asset.intelligence.has_audio || track_id == "a1" {
                speech_present = true;
            }
            if let Some(lufs) = asset.intelligence.integrated_lufs {
                lufs_checked = true;
                let effective = lufs + clip.audio.gain_db;
                if (effective - (-14.0)).abs() > 2.5 {
                    let target_gain = ((-14.0 - lufs).clamp(-12.0, 12.0) * 100.0).round() / 100.0;
                    // Avoid infinite QA auto-fix loops when gain is already at the +/-12 dB clamp
                    if auto_fix && (clip.audio.gain_db - target_gain).abs() >= 0.05 {
                        let mut next_audio = clip.audio.clone();
                        next_audio.gain_db = target_gain;
                        let fixed = apply_locked_op(
                            backend,
                            &project.id,
                            track_id,
                            TimelineCommand::SetAudioSettings {
                                track_id: track_id.to_string(),
                                clip_id: clip.id.clone(),
                                audio: next_audio.clone(),
                            },
                            &format!("Normalize clip '{}' to -14 LUFS", clip.id),
                            ops_counter,
                        )
                        .await;
                        if fixed {
                            any_mutated = true;
                            *auto_fixes_applied += 1;
                        }
                        checks.push(QaCheckItem {
                            id: format!("lufs-{}", clip.id),
                            category: "audio".to_string(),
                            severity: if fixed {
                                QaSeverity::Fixed
                            } else {
                                QaSeverity::Warning
                            },
                            message: if fixed {
                                format!(
                                    "Adjusted '{}' gain to {:.1} dB toward -14 LUFS (source {lufs:.1} LUFS).",
                                    clip.label, next_audio.gain_db
                                )
                            } else {
                                format!(
                                    "Clip '{}' effective loudness {effective:.1} LUFS deviates from -14 LUFS.",
                                    clip.label
                                )
                            },
                            time_sec: Some(clip.start_sec),
                            auto_fixed: fixed,
                        });
                    } else {
                        checks.push(QaCheckItem {
                            id: format!("lufs-{}", clip.id),
                            category: "audio".to_string(),
                            severity: QaSeverity::Warning,
                            message: format!(
                                "Clip '{}' effective loudness {effective:.1} LUFS deviates from -14 LUFS target (gain clamped at {:.1} dB).",
                                clip.label, clip.audio.gain_db
                            ),
                            time_sec: Some(clip.start_sec),
                            auto_fixed: false,
                        });
                    }
                }
            }
        }
    }

    if lufs_checked && !checks.iter().any(|c| c.id.starts_with("lufs-")) {
        checks.push(QaCheckItem {
            id: "lufs-target".to_string(),
            category: "audio".to_string(),
            severity: QaSeverity::Pass,
            message: "Dialogue/A-Roll levels are within -14 LUFS broadcast tolerance.".to_string(),
            time_sec: None,
            auto_fixed: false,
        });
    }

    if speech_present && let Some(a2) = project.tracks.iter().find(|t| t.id == "a2" && !t.muted) {
        for music in &a2.clips {
            if cancel.is_cancelled() {
                return any_mutated;
            }
            if music.audio.gain_db <= -90.0 {
                continue;
            }
            let needs_duck = !music.audio.ducking_enabled
                || music.audio.duck_target_db > -12.0
                || music.audio.gain_db > -8.0;
            if needs_duck {
                if auto_fix {
                    let mut next_audio = music.audio.clone();
                    next_audio.ducking_enabled = true;
                    next_audio.duck_target_db = -18.0;
                    next_audio.gain_db = next_audio.gain_db.min(-12.0);
                    let fixed = apply_locked_op(
                        backend,
                        &project.id,
                        "a2",
                        TimelineCommand::SetAudioSettings {
                            track_id: "a2".to_string(),
                            clip_id: music.id.clone(),
                            audio: next_audio,
                        },
                        "Enable -18 dB music ducking under speech",
                        ops_counter,
                    )
                    .await;
                    if fixed {
                        any_mutated = true;
                        *auto_fixes_applied += 1;
                    }
                    checks.push(QaCheckItem {
                        id: format!("ducking-{}", music.id),
                        category: "audio".to_string(),
                        severity: if fixed {
                            QaSeverity::Fixed
                        } else {
                            QaSeverity::Warning
                        },
                        message: if fixed {
                            format!(
                                "Enabled -18 dB sidechain ducking on music '{}'.",
                                music.label
                            )
                        } else {
                            format!("Music '{}' lacks -18 dB ducking under speech.", music.label)
                        },
                        time_sec: Some(music.start_sec),
                        auto_fixed: fixed,
                    });
                } else {
                    checks.push(QaCheckItem {
                        id: format!("ducking-{}", music.id),
                        category: "audio".to_string(),
                        severity: QaSeverity::Warning,
                        message: format!(
                            "Music '{}' lacks -18 dB ducking under speech.",
                            music.label
                        ),
                        time_sec: Some(music.start_sec),
                        auto_fixed: false,
                    });
                }
            }
        }
    }

    any_mutated
}

pub(super) async fn check_and_fix_safe_zones(
    backend: &dyn SolarisBackend,
    project: &mut Project,
    auto_fix: bool,
    cancel: &CancellationToken,
    ops_counter: Option<&AtomicU32>,
    auto_fixes_applied: &mut u32,
    checks: &mut Vec<QaCheckItem>,
) -> bool {
    let mut any_mutated = false;
    let x_limit = if project.canvas.aspect_preset == AspectPreset::Vertical9x16 {
        0.2
    } else {
        0.35
    };

    if let Some(t1) = project.tracks.iter().find(|t| t.id == "t1" && !t.muted) {
        for cap in &t1.clips {
            if cancel.is_cancelled() {
                return any_mutated;
            }
            if let ClipKind::Caption { caption } = &cap.payload
                && caption.words.len() > 6
            {
                checks.push(QaCheckItem {
                    id: format!("cap-density-{}", cap.id),
                    category: "safe_zone".to_string(),
                    severity: QaSeverity::Warning,
                    message: format!(
                        "Caption '{}' has {} words; keep bursts to 2-4 words for readability.",
                        cap.label,
                        caption.words.len()
                    ),
                    time_sec: Some(cap.start_sec),
                    auto_fixed: false,
                });
            }

            let out_of_bounds = cap.transform.y < -0.35
                || cap.transform.y > 0.35
                || cap.transform.x.abs() > x_limit
                || cap.transform.scale < 0.75
                || cap.transform.scale > 1.35
                || cap.transform.opacity < 0.85;

            if out_of_bounds {
                if auto_fix {
                    let mut next = cap.transform.clone();
                    next.y = next.y.clamp(-0.32, 0.32);
                    next.x = next.x.clamp(-x_limit, x_limit);
                    next.scale = next.scale.clamp(0.85, 1.25);
                    next.opacity = next.opacity.max(0.95);
                    let fixed = apply_locked_op(
                        backend,
                        &project.id,
                        "t1",
                        TimelineCommand::UpdateClipTransform {
                            track_id: "t1".to_string(),
                            clip_id: cap.id.clone(),
                            transform: next,
                        },
                        "Clamp caption into social safe-zone",
                        ops_counter,
                    )
                    .await;
                    if fixed {
                        any_mutated = true;
                        *auto_fixes_applied += 1;
                    }
                    checks.push(QaCheckItem {
                        id: format!("safezone-{}", cap.id),
                        category: "safe_zone".to_string(),
                        severity: if fixed {
                            QaSeverity::Fixed
                        } else {
                            QaSeverity::Warning
                        },
                        message: if fixed {
                            format!("Moved caption '{}' inside 80% social safe zone.", cap.label)
                        } else {
                            format!("Caption '{}' sits outside 80% social safe zone.", cap.label)
                        },
                        time_sec: Some(cap.start_sec),
                        auto_fixed: fixed,
                    });
                } else {
                    checks.push(QaCheckItem {
                        id: format!("safezone-{}", cap.id),
                        category: "safe_zone".to_string(),
                        severity: QaSeverity::Warning,
                        message: format!(
                            "Caption '{}' sits outside 80% social safe zone.",
                            cap.label
                        ),
                        time_sec: Some(cap.start_sec),
                        auto_fixed: false,
                    });
                }
            }
        }
    }

    // Check G1 GFX safe-zone bounds too
    if let Some(g1) = project.tracks.iter().find(|t| t.id == "g1" && !t.muted) {
        for gfx in &g1.clips {
            if cancel.is_cancelled() {
                return any_mutated;
            }
            let out_of_bounds = gfx.transform.y < -0.35
                || gfx.transform.y > 0.35
                || gfx.transform.x.abs() > x_limit
                || gfx.transform.scale < 0.5
                || gfx.transform.scale > 1.5;
            if out_of_bounds {
                if auto_fix {
                    let mut next = gfx.transform.clone();
                    next.y = next.y.clamp(-0.32, 0.32);
                    next.x = next.x.clamp(-x_limit, x_limit);
                    next.scale = next.scale.clamp(0.6, 1.35);
                    let fixed = apply_locked_op(
                        backend,
                        &project.id,
                        "g1",
                        TimelineCommand::UpdateClipTransform {
                            track_id: "g1".to_string(),
                            clip_id: gfx.id.clone(),
                            transform: next,
                        },
                        "Clamp GFX overlay into social safe-zone",
                        ops_counter,
                    )
                    .await;
                    if fixed {
                        any_mutated = true;
                        *auto_fixes_applied += 1;
                    }
                    checks.push(QaCheckItem {
                        id: format!("safezone-gfx-{}", gfx.id),
                        category: "safe_zone".to_string(),
                        severity: if fixed {
                            QaSeverity::Fixed
                        } else {
                            QaSeverity::Warning
                        },
                        message: if fixed {
                            format!("Moved GFX '{}' inside 80% social safe zone.", gfx.label)
                        } else {
                            format!("GFX '{}' sits outside 80% social safe zone.", gfx.label)
                        },
                        time_sec: Some(gfx.start_sec),
                        auto_fixed: fixed,
                    });
                } else {
                    checks.push(QaCheckItem {
                        id: format!("safezone-gfx-{}", gfx.id),
                        category: "safe_zone".to_string(),
                        severity: QaSeverity::Warning,
                        message: format!("GFX '{}' sits outside 80% social safe zone.", gfx.label),
                        time_sec: Some(gfx.start_sec),
                        auto_fixed: false,
                    });
                }
            }
        }
    }

    // Refresh project snapshot before overlap detection so we check against updated coordinates.
    if any_mutated && let Ok(refreshed) = backend.get_project(&project.id).await {
        *project = refreshed;
    }

    if let (Some(t1), Some(g1)) = (
        project.tracks.iter().find(|t| t.id == "t1" && !t.muted),
        project.tracks.iter().find(|t| t.id == "g1" && !t.muted),
    ) {
        for gfx in &g1.clips {
            if cancel.is_cancelled() {
                return any_mutated;
            }
            let gfx_end = gfx.start_sec + gfx.duration_sec;
            let overlapping_caps: Vec<&Clip> = t1
                .clips
                .iter()
                .filter(|cap| {
                    let cap_end = cap.start_sec + cap.duration_sec;
                    gfx.start_sec < cap_end && cap.start_sec < gfx_end
                })
                .collect();
            let colliding_cap = overlapping_caps
                .iter()
                .copied()
                .find(|cap| (gfx.transform.y - cap.transform.y).abs() < 0.08);
            if let Some(cap) = colliding_cap {
                let primary_candidate = if cap.transform.y >= 0.0 { -0.22 } else { 0.22 };
                let candidates = [primary_candidate, -primary_candidate, 0.0, -0.30, 0.30];
                // Pick the first vertical band that does not collide with ANY overlapping caption
                let target_y = candidates
                    .into_iter()
                    .find(|&cand| {
                        overlapping_caps
                            .iter()
                            .all(|other| (cand - other.transform.y).abs() >= 0.08)
                    })
                    .unwrap_or(primary_candidate);
                if auto_fix {
                    let mut next = gfx.transform.clone();
                    next.y = target_y;
                    let fixed = apply_locked_op(
                        backend,
                        &project.id,
                        "g1",
                        TimelineCommand::UpdateClipTransform {
                            track_id: "g1".to_string(),
                            clip_id: gfx.id.clone(),
                            transform: next,
                        },
                        "Offset GFX to non-colliding vertical band from captions",
                        ops_counter,
                    )
                    .await;
                    if fixed {
                        any_mutated = true;
                        *auto_fixes_applied += 1;
                    }
                    checks.push(QaCheckItem {
                        id: format!("overlap-{}", gfx.id),
                        category: "safe_zone".to_string(),
                        severity: if fixed {
                            QaSeverity::Fixed
                        } else {
                            QaSeverity::Warning
                        },
                        message: if fixed {
                            format!(
                                "Repositioned GFX '{}' to band ({target_y:.2}) to avoid caption collision.",
                                gfx.label
                            )
                        } else {
                            format!(
                                "GFX '{}' overlaps caption band at {:.2}s.",
                                gfx.label, gfx.start_sec
                            )
                        },
                        time_sec: Some(gfx.start_sec),
                        auto_fixed: fixed,
                    });
                } else {
                    checks.push(QaCheckItem {
                        id: format!("overlap-{}", gfx.id),
                        category: "safe_zone".to_string(),
                        severity: QaSeverity::Warning,
                        message: format!(
                            "GFX '{}' overlaps caption band at {:.2}s.",
                            gfx.label, gfx.start_sec
                        ),
                        time_sec: Some(gfx.start_sec),
                        auto_fixed: false,
                    });
                }
            }
        }
    }

    if !checks.iter().any(|c| c.category == "safe_zone") {
        checks.push(QaCheckItem {
            id: "safezone-clean".to_string(),
            category: "safe_zone".to_string(),
            severity: QaSeverity::Pass,
            message: "Captions and GFX overlays respect social safe-zones without collision."
                .to_string(),
            time_sec: None,
            auto_fixed: false,
        });
    }

    any_mutated
}

pub(super) fn check_gfx_lint(
    project: &Project,
    assets: &[SavedCodeAsset],
    checks: &mut Vec<QaCheckItem>,
) {
    let Some(g1) = project.tracks.iter().find(|t| t.id == "g1") else {
        return;
    };
    let mut gfx_count = 0usize;
    let mut error_count = 0usize;

    for clip in &g1.clips {
        let ClipKind::Gfx { gfx } = &clip.payload else {
            continue;
        };
        gfx_count += 1;
        let html = gfx.inline_html.as_deref().or_else(|| {
            gfx.asset_id
                .as_deref()
                .and_then(|aid| assets.iter().find(|a| a.id == aid))
                .map(|a| a.html_source.as_str())
        });
        if let Some(source) = html {
            let report = lint_gfx_html(source);
            if !report.valid {
                error_count += 1;
                checks.push(QaCheckItem {
                    id: format!("gfx-lint-{}", clip.id),
                    category: "gfx".to_string(),
                    severity: QaSeverity::Issue,
                    message: format!(
                        "GFX '{}' failed deterministic HTML lint: {}",
                        clip.label,
                        report.errors.join("; ")
                    ),
                    time_sec: Some(clip.start_sec),
                    auto_fixed: false,
                });
            }
        }
    }

    if gfx_count > 0 && error_count == 0 {
        checks.push(QaCheckItem {
            id: "gfx-lint-clean".to_string(),
            category: "gfx".to_string(),
            severity: QaSeverity::Pass,
            message: format!("All {gfx_count} GFX overlay(s) pass deterministic HTML/GSAP lint."),
            time_sec: None,
            auto_fixed: false,
        });
    }
}
