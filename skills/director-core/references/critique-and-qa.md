# Closed Multimodal QA & Critique Loop

## Table of Contents

1. [Exact Automated QA Thresholds](#1-exact-automated-qa-thresholds)
2. [Multi-Timestamp Visual Sampling Strategy](#2-multi-timestamp-visual-sampling-strategy)
3. [6-Axis Director Critique Rubric (1–10)](#3-6-axis-director-critique-rubric-110)
4. [Fix-Line Contract & Max-3-Round Loop](#4-fix-line-contract--max-3-round-loop)

---

## 1. Exact Automated QA Thresholds

`project.run_qa` (`auto_fix: true | false`) checks these exact deterministic thresholds in `crates/agent-runtime/src/swarm/qa/checks.rs`:

| Check | Exact Pass Threshold | What `auto_fix: true` Repairs |
| --- | --- | --- |
| **Visuals Presence (`visuals-empty`)** | At least 1 active unmuted visual clip on `V1`, `V2`, or `G1` (`duration_sec > 0`) | Flags hard `issue` (`-40` pts, caps report score `<= 65`) if program canvas is completely blank behind captions/audio. |
| **Programmatic G1 Coverage (`g1-coverage-low`)** | When no `V1`/`V2` clips exist and active timeline `>= 2.0s`, `G1` must cover `>= 70%` of unmuted timeline duration | Flags hard `issue` if large blank gaps remain on `G1`. |
| **Hook Lead Gap** | First visual clip on `V1`/`V2` (or `G1` when no `V1`/`V2` clips exist) starts at `<= 0.50s` | Flags `warning` if lead gap `> 0.50s` (shift spine or opening `G1` scene manually). |
| **Primary Track Continuity Gaps** | No uncovered gap `> 0.40s` between consecutive clips on `V1` (unless covered by `V2`), or on `G1` in programmatic mode | Flags `warning` (trim/close gap on `V1`/`G1` or cover `V1` with `V2`). |
| **Caption Safe Zone (`T1`)** | `y ∈ [-0.35, 0.35]`; `x ∈ [-0.20, 0.20]` (9:16) or `[-0.35, 0.35]` (16:9/1:1); `scale ∈ [0.75, 1.35]`; `opacity >= 0.85`; `words <= 6` | Clamps out-of-bounds `x`, `y`, `scale`, and `opacity`. |
| **GFX Safe Zone (`G1`)** | `y ∈ [-0.35, 0.35]`; `x ∈ [-0.20, 0.20]` (9:16) or `[-0.35, 0.35]` (16:9/1:1); `scale ∈ [0.50, 1.50]` | Clamps out-of-bounds `x`, `y`, and `scale`. |
| **T1 / G1 Collision** | Overlapping floating `G1` & `T1` clips must satisfy `|gfx.y - cap.y| >= 0.08` (**full-canvas programmatic `G1` scenes are exempt** and stay at `(0,0)`) | Auto-offsets colliding floating `G1` clip to a collision-free vertical band (`±0.22`, `0.0`, `±0.30`), or warns if no free band exists. |
| **GFX Sandbox Lint** | Every `G1` clip passes `gfx.lint_html` (`class="clip"`, `data-start`, `data-duration`, no forbidden APIs/URLs) | Flags hard `issue` (must fix HTML and replace clip). |
| **Voice Loudness (`V1`/`A1`)** | Effective loudness (`integrated_lufs + gain_db`) within `±2.5 LUFS` of `-14.0` (skips muted tracks & clips `<= -90 dB`; gain clamped to `[-12, +12]` dB) | Applies gain delta toward `-14.0 LUFS` within `±12 dB` clamp. |
| **Music Ducking (`A2`)** | When speech exists: `ducking_enabled: true`, `duck_target_db <= -12.0`, AND `gain_db <= -8.0` | Enables ducking (`-18.0 dB`) and lowers excessive `gain_db` to `-12.0 dB`. |

---

## 2. Multi-Timestamp Visual Sampling Strategy

Never judge a timeline from a single midpoint frame—most visual defects happen at entrances, transitions, and overlay overlaps. Call `preview.seek` + `preview.capture_frame` at **3 to 5 deliberate timestamps**:

1. **Hook (`0.3–0.6s`)**: Confirm frame is populated (no black start), eyes/subject sit cleanly in upper third, and hook title does not cover the speaker's face.
2. **First Overlay Overlap**: Pick a timestamp where `T1` captions AND a `G1` card are simultaneously active. Confirm clean vertical separation, high contrast, and zero clipping at edges.
3. **B-Roll / Cutaway Midpoint**: Confirm `V2` framing, `9:16` crop alignment, and color match against `V1`.
4. **Climax / Stat Reveal**: Verify numbers/labels are readable at mobile scale (equivalent to 360px phone width) and brand tokens match `brand.get_playbook`.
5. **Outro / Last Second**: Confirm clean end framing without frozen/orphaned overlays.

> **Prerequisite**: `preview.capture_frame` works for **both** `V1`/`V2` footage timelines AND programmatic `G1`/`T1` overlay-only timelines (rendered over the brand backdrop). Headless Chromium is required to render `G1`/`T1` overlays; if no active clip exists on `V1`/`V2`/`G1`/`T1` at `time_sec`, pick a covered timestamp or fill the gap first.

---

## 3. 6-Axis Director Critique Rubric (1–10)

Be a harsh supervising editor, not a proud author. **Any unresolved measured QA issue in an axis hard-caps that axis at `7` until fixed.**

| Axis | 9–10 (Release Ready) | 7–8 (Solid, Minor Tweak) | 5–6 (Visible Amateur Tell) | < 5 (Broken / Unshippable) |
| --- | --- | --- | --- | --- |
| **Hook** | Frame 0 immediately delivers subject + visual/verbal hook; strong framing in first 2s | Hook lands cleanly within 0.5s | Slow buildup, generic title on blank frame | Black lead gap `>0.5s` or dead air |
| **Brand** | Matches `brand_md` tokens, fonts, LUT intensity, and tone effortlessly | Right palette, one off-brand graphic or font mismatch | Generic stock look ignoring playbook | Wrong palette, placeholder text, unrecovered Log gray |
| **Pacing** | Tight breath-safe cuts, B-roll every 3–5s, zero static stretches `>3.5s`, beat-aligned | One slightly long hold or loose cut | Repeated dead pauses or static holds `>4s` | Uncut raw footage or broken V1 gaps `>0.4s` |
| **Readability** | 2–4 word bursts, AA contrast, clear of 9:16 chrome, zero T1/G1 collision, legible at phone width | One cramped line or tight edge margin | Overlaps face/chrome or 6+ word walls | Cut-off text, colliding overlays, unreadable color |
| **Color & Framing** | Natural skin tones, clean shadow/highlight roll-off, intentional punch-ins, balanced V1/V2 match | Minor warmth/contrast mismatch on one cutaway | Flat Log footage or over-saturated 100% LUT crush | Clipped highlights, misaligned 9:16 crop cutting face |
| **Audio & Polish** | Voice `-14 ± 2.5 LUFS`, music `<= -8 dB` + `-18 dB` ducking, click-free fades, all GFX lint-clean | Minor fade missing on one clip | Music too loud or ducking disabled | Severe loudness drift, unmuted clashing B-roll, lint errors |

---

## 4. Fix-Line Contract & Max-3-Round Loop

1. After inspecting QA JSON and captured PNG frames, list every axis scoring `< 8` with an explicit fix line:
   ```text
   t=4.2s · g1 (stat-1) · overlaps T1 caption at y=0.18 -> timeline.set_transform(track_id="g1", clip_id="stat-1", y=-0.24)
   t=0.4s · fx (global) · S-Log3 footage still washed out -> color.set_grade(contrast=1.18, saturation=1.16, lut_intensity=0.75)
   ```
2. **Scope discipline**: Apply *only* the listed fixes—do not randomly rebuild working clips.
3. Re-run `project.run_qa` and re-capture only the affected timestamps to confirm the fix resolved the issue without regressions.
4. If a capture looks worse than before, revert immediately via `project.undo`.
5. **Hard cap: maximum 3 QA rounds.** After Round 3, deliver the best revision and report transparently any axis still `< 8` plus the exact reason.
6. **Audio disclosure**: Always note in the final summary that audio loudness (`LUFS`) and ducking were verified via numeric telemetry/mix settings, not listened to by ear.
