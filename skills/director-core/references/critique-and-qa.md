# Closed Multimodal QA & Critique Loop

## Table of Contents

1. [Exact Automated QA Thresholds](#1-exact-automated-qa-thresholds)
2. [Multi-Timestamp Visual Sampling Strategy](#2-multi-timestamp-visual-sampling-strategy)
3. [6-Axis Director Critique Rubric (1–10)](#3-6-axis-director-critique-rubric-110)
4. [Repeated Review Notes → Exact Mechanism Diagnosis](#4-repeated-review-notes--exact-mechanism-diagnosis)
5. [Scoped Fix-Line Contract & Max-3-Round Loop](#5-scoped-fix-line-contract--max-3-round-loop)

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

Never judge a timeline from a single midpoint frame—most defects live at entrances, transitions, and overlay overlaps. Call `preview.seek` + `preview.capture_frame` at **3 to 5 deliberate timestamps**:

1. **Hook (`0.3–0.6s`)**: Confirm frame is populated with strong ink (no black start), eyes/subject sit cleanly in upper third, and hook title does not cover the speaker's face.
2. **First Overlay Overlap**: Pick a timestamp where `T1` captions AND a `G1` card are simultaneously active. Confirm clean vertical separation, high contrast, and zero edge clipping.
3. **B-Roll / Programmatic Midpoint**: Confirm `V2` framing / `9:16` crop alignment / color match, OR in programmatic `G1` verify the full `100vh` atmosphere is alive (no pitch-black bottom void).
4. **Climax / Stat Reveal**: Verify numbers and labels are legible at **360px phone width** and brand tokens match `brand.get_playbook`.
5. **Outro / Last Second**: Confirm clean end framing without frozen/orphaned overlays or premature fade-to-black.

### The Stranger & Mute Tests (Apply to Every Captured Frame)
1. **Describe the frame in one sentence as a stranger would**, naming every object. Anything you would have to explain is a design flaw.
2. **Cover the words**: Do the visuals alone still communicate the beat (trend, comparison, proof, action)?
3. **Phone-width test**: Imagine the frame shrunk to `360px` wide on a phone, muted. Can every headline, stat, and caption be read cleanly in under a second?

> **Prerequisite**: `preview.capture_frame` works for **both** `V1`/`V2` footage timelines AND programmatic `G1`/`T1` overlay-only timelines (rendered over the brand backdrop). Headless Chromium is required to render `G1`/`T1` overlays.

---

## 3. 6-Axis Director Critique Rubric (1–10)

Be the harsh supervising motion director, not the proud author. **Any unresolved measured QA issue in an axis hard-caps that axis at `7` until fixed.**

| Axis | 9–10 (Release Ready) | 7–8 (Solid, Minor Tweak) | 5–6 (Visible Amateur Tell) | < 5 (Broken / Unshippable) |
| --- | --- | --- | --- | --- |
| **Hook** | Frame 0 immediately delivers subject/result + bold hook; first 2s pose a clear question or change | Hook lands cleanly within 0.5s | Slow buildup, generic title fading over flat ground | Black lead gap `>0.5s` or dead air |
| **Brand** | Matches `brand_md` tokens, proportions, fonts, LUT intensity, and cohesive world effortlessly | Right palette, one off-brand graphic or font mismatch | Generic "tech" look ignoring playbook | Wrong palette, placeholder text, internal asset IDs visible, raw Log gray |
| **Motion & Pacing** | Role-based eases, arrive slow / leave clean, tight breath-safe cuts, B-roll/beats every 0.5–3.5s, zero static holds `>3.5s` | One slightly long hold, loose cut, or floaty element | Everything fades-and-slides up on one ease; dead stretches `>4s` | Uncut raw footage, V1/G1 gaps `>0.4s`, jitter or stale blur |
| **Readability** | Every line held `0.5s + 0.3s/word`, readable at 360px phone width, AA contrast, clear of 9:16 chrome, zero T1/G1 collision | One cramped line or tight edge margin | Overlaps face/chrome, tiny SVG labels, or 6+ word walls | Cut-off text, colliding overlays, unreadable color |
| **Color & Framing** | Natural skin tones, clean shadow/highlight roll-off, intentional punch-ins, balanced V1/V2 match, hero fills 40%+ of stage | Minor warmth/contrast mismatch on one cutaway | Flat Log footage, 100% LUT crush, or tiny box in empty frame | Clipped highlights, misaligned 9:16 crop cutting face |
| **Audio & Polish** | Voice `-14 ± 2.5 LUFS`, music `<= -8 dB` + `-18 dB` ducking, click-free fades, shadow-safe clips, all GFX lint-clean, held end card | Minor fade missing or one small blemish | Music too loud, ducking disabled, or several visible blemishes | Severe loudness drift, unmuted clashing B-roll, lint errors, broken frames |

---

## 4. Repeated Review Notes → Exact Mechanism Diagnosis

When one of these real-world defects appears on a captured frame, stop tweaking colors and fix the exact underlying mechanism:

| Visual Defect Seen in Capture | Root Mechanism | Exact Solaris Fix |
| --- | --- | --- |
| **"Too much empty dark screen"** | Hero element too small or ambient backdrop stops at `50vh`. | Scale hero to fill `40%+` of stage; extend radial glow + subtle grid across full `100vh`. |
| **"Empty at the start"** | Lead gap `>0.5s` or opening `G1` fades in from `opacity: 0`. | Start `V1`/`G1` at `0.0s` with frame-0 ink (`opacity >= 0.85` at `t=0`). |
| **"Drop shadow is a sharp box"** | `clip-path` wipe at `inset(0 ...)` clipped a shadowed element. | Use shadow-safe negative insets (`inset(-40% 100% -40% -10%)`) or `filter: drop-shadow(...)`. |
| **"Letters or glow cut off at bottom"** | Mask floor sits at the line box (`0%`) instead of below descenders. | Use `.mask` with `padding-bottom: 0.42em; margin-bottom: -0.42em; clip-path: inset(-30% -10% 0% -10%)`. |
| **"Hairline seam through letters"** | `letter-spacing` tighter than `-0.025em`. | Clamp `letter-spacing >= -0.025em`. |
| **"Stale blur frozen on card"** | `filter: blur()` tweened with `back.out` / `elastic` ease. | Move `back.out` to transform only; tween `blur` and `opacity` with `power2.out` / `expo.out`. |
| **"Earlier frame shows exit state"** | Exit or repeated `fromTo` missing `immediateRender: false`. | Add `immediateRender: false` to all exits and repeated tweens. |
| **"Counter shows $0 before counting"** | Counter text pre-rendered at `t=0`. | Hide with `tl.set(el, { opacity: 0 }, 0)` until count start `t`, then count + stamp. |
| **"Boxes move robotically"** | Uniform sequential stagger or per-item wobble loop. | Overlap entrances 30–50%, vary eases by role, freeze landed titles. |
| **"GFX collides with T1 captions"** | Floating `G1` left at default `y: 0` or full-canvas content spilled below `61vh`. | Offset floating `G1` via `timeline.set_transform(y=-0.24)`; keep 9:16 full-canvas foreground inside `5vh..61vh`. |
| **"Useless progress bars or `01/06` / `ETAPA 02` counters"** | Slide-deck chrome stamped onto video scenes. | Remove unrequested bottom loading/progress bars and scene/step counters (`01/06`, `ETAPA 02`, `STEP 2`) via `gfx.update_clip`; let the visual storytelling carry the frame. |
| **"Badge/label clipped or overlapping text"** | Absolute-positioned badge with negative `right`/`top` offsets collides with giant 9:16 headline or overflows card padding. | Move the badge out of `position: absolute` into normal CSS Flex/Grid flow (above the headline in the eyebrow row or inline with explicit `gap`) and expand container padding via `gfx.update_clip`. |
| **"Card icons stretched or floating on top of card text"** | Descendant CSS selector (e.g., `.funnel svg { position: absolute; inset: 0; width: 100%; height: 100%; }`) captures nested icon `<svg>` elements inside GSAP-transformed cards. | Change the backdrop rule to a direct-child selector (`.funnel > svg`) or explicit class (`.funnel-bg`) so nested icon `<svg>` elements stay inside their icon box. |
| **"SVG node/dot covers text or labels collide with strokes/shield"** | Hand-placed `<text>` or dots overlap SVG geometry or enclosing outlines. | Move multi-word labels/cards to CSS Flex/Grid containers outside the SVG, offset short SVG labels outside circle radii (`>= 16px`), remove dots over text, and keep `>= 32px` padding inside enclosing shields/rings. |
| **"Empty placeholder node rings with orphan labels"** | Hollow `<circle>` nodes drawn without icons or inner cores. | Populate every node circle with a crisp linear icon or solid glowing core dot and center-align its label cleanly below/above the ring. |
| **"Scene freezes dead for > 3s after entrance"** | Timeline ends all tweens at `t=1.0s` on a `5–7s` clip. | Add continuous GSAP mid-scene motion (`stroke-dashoffset` edge pulses, slow orbital drift, or pulsing alert rings) spanning `t=0.8s..duration` while keeping headlines still. |
| **"Empty space inside card or blank lower stage even though DOM elements exist"** | Delayed GSAP entrances still at `opacity: 0` at the sampled timestamp (e.g. starting at `2.15s–2.75s` on a `4.0s` clip). | Overlap/tighten GSAP start times so all core + footer elements finish entering by `<= 45%` of `duration_sec` (max `~1.8s`) instead of stretching card height. |

---

## 5. Scoped Fix-Line Contract & Max-3-Round Loop

1. After inspecting QA JSON and captured PNG frames, list every axis scoring `< 8` with an explicit fix line:
   ```text
   t=4.2s · g1 (stat-1) · overlaps T1 caption at y=0.18 -> timeline.set_transform(track_id="g1", clip_id="stat-1", y=-0.24)
   t=0.4s · fx (global) · S-Log3 footage still washed out -> color.set_grade(contrast=1.18, saturation=1.16, lut_intensity=0.75)
   ```
2. **Scoped fix discipline ("change nothing else")**: Apply *only* the listed fixes—never randomly rebuild working clips. Fixed mappings:
   - *"too fast"* → slow camera/entrance durations by `1.3×` and change nothing else.
   - *"feels like slides"* → add a continuous rig drift, specular sweep, or object handoff and change nothing else.
   - *"titles move"* → freeze landed titles dead still and change nothing else.
3. Re-run `project.run_qa` and re-capture only the affected timestamps to confirm the fix resolved the issue without regressions.
4. If a capture looks worse than before, revert immediately via `project.undo`.
5. **Hard cap: maximum 3 QA rounds.** After Round 3, deliver the best revision and report transparently any axis still `< 8` plus the exact reason.
6. **Audio disclosure**: Always note in the final summary that audio loudness (`LUFS`) and ducking were verified via numeric telemetry/mix settings, not listened to by ear.
