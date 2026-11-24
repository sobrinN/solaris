---
name: faceless-explainer
description: Builds high-retention faceless explainers, product demos, and narrated essays in Solaris (16:9 or 9:16) with under 3.5s visual density, beat-synced B-roll cuts, Ken Burns still motion, and structured HTML/GSAP callout cards. Use when assembling voiceover-driven videos without an on-camera host, product launches, SaaS walkthroughs, or narrated visual stories. Not for on-camera talking-head jump cuts.
---

# Solaris Faceless Explainer Craft

Use this skill when the video is driven by voiceover (`A1`) + visual storytelling (`V1`, `V2`, `G1`, `T1`) rather than an on-camera speaker.

## 1. Pre-Edit Intelligence & Beat Mapping

1. Call `media.get_manifest` on every narration (`A1`), music (`A2`), B-roll (`V1`/`V2`), and still image asset.
2. If the narration asset (`role_hint: "voice"`) has an empty `transcript`, call `media.transcribe` first—narration phrase timestamps anchor every visual beat and caption.
3. Inspect `beats` on the `A2` music track and `scene_cuts` on multi-shot B-roll videos so cuts land naturally on musical downbeats rather than mid-shot jumps.
4. Call `media.extract_keyframes` on unfamiliar video clips to verify visual subjects match the narrated sentence.

## 2. Spine & Visual Density Contract (`A1` + `V1` + `V2`)

- **Audio Spine First (`A1`)**: Place voiceover on **`a1`** (never `a2`, which is reserved for ducked music). Trim dead air `> 0.35s` with `0.12s` breath pads while keeping a natural documentary cadence.
- **Zero Static Stretches (`<= 3.5s` Rule)**: Never hold a single unchanged visual on screen for more than **`3.5s`**. Change camera angle, cut to a fresh `V1`/`V2` clip, trigger a `G1` stat/callout reveal, or shift visual focal framing.
- **Mandatory Ken Burns on Stills**: Whenever placing a `MediaKind::Image` asset via `timeline.place_clip`, always set **`ken_burns: true`** so Solaris applies smooth centered zoom/pan motion in both WebGL2 preview and FFmpeg export.
- **Zero Visual Gaps**: Keep `V1` continuously covered from `0.0s` through the end of narration (`first visual <= 0.50s`, no uncovered gaps `> 0.40s`). Clamp `A2` music duration to the narration/visual end with a clean `0.8s` fade-out.
- **Beat-Synced Cutting**: Snap `V1`/`V2` cuts and `G1` entrances within **`±0.10s`** of `manifest.beats` or narration sentence starts.

## 3. Structured Graphics Hierarchy (`G1` + `T1`)

1. **Always search before authoring**: Call `assets.search` first and reuse Solaris's 5 starter cards (`chapter-title-solar`, `stat-card-solar`, `lower-third-solar`, `comment-bubble-solar`, `progress-bar-solar`) before writing custom HTML.
2. **One Idea Per Visual Beat**:
   - **Hook (`0.0–2.5s`)**: Open directly on the strongest visual or result + `chapter-title-solar` (positioned cleanly via `timeline.set_transform` on `g1`, e.g. `y: -0.18`).
   - **Proof / Metrics**: Land `stat-card-solar` within `0.10s` of every spoken statistic—**only use real numbers from the script/transcript**.
   - **Steps / Progression**: Use `progress-bar-solar` (`label: "STEP 01 / 03"`, `percent: 33`) at the top safe band (`y: -0.30`) to telegraph structure.
   - **Context / Attribution**: Use `lower-third-solar` for product names, quotes, or chapters, offset cleanly away from `T1` captions (`|gfx.y - cap.y| >= 0.08`).
3. **Captions Alignment**: Call `captions.generate_from_transcript` after `A1` is placed (Solaris maps word timestamps against both `V1` and `A1`). For `16:9` explainers, prefer `minimal` or `mono_caps` at `3–4` words/burst unless the user requested high-energy `neon_glow`/`karaoke`.

## 4. BANNED Faceless Explainer Defaults

- Placing voiceover audio onto `A2` (gets ducked as music!).
- Placing still images with `ken_burns: false` (reads as a dead PowerPoint slide).
- Leaving a static visual on screen for `> 3.5s` with no cut, reveal, or motion.
- Letting `A2` music run 30 seconds past the final visual clip or drowning narration without `ducking_enabled: true`.
- Stacking `G1` overlays on top of `T1` captions or inventing statistics not present in the script.
