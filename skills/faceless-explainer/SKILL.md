---
name: faceless-explainer
description: Builds high-retention faceless explainers, product demos, programmatic zero-asset videos, and narrated essays in Solaris (16:9 or 9:16) with under 3.5s visual density, beat-synced B-roll cuts, Ken Burns still motion, cohesive Visual Worlds, and structured HTML/CSS/SVG/GSAP scenes. Use when assembling voiceover-driven videos without an on-camera host, product launches, SaaS walkthroughs, or narrated visual stories. Not for on-camera talking-head jump cuts.
---

# Solaris Faceless Explainer & Programmatic Film Craft

Use this skill when the video is driven by voiceover (`A1`) + visual storytelling (`V1`, `V2`, `G1`, `T1`)—or when building a **zero-asset programmatic video** entirely on `G1`—rather than an on-camera speaker.

## 1. Pre-Edit Intelligence & Beat Mapping

1. Call `media.get_manifest` on every narration (`A1`), music (`A2`), B-roll (`V1`/`V2`), and still image asset.
2. If the narration asset (`role_hint: "voice"`) has an empty `transcript`, call `media.transcribe` first—narration phrase timestamps anchor every visual beat and caption.
3. Inspect `beats` on the `A2` music track and `scene_cuts` on multi-shot B-roll videos so cuts land naturally on musical downbeats rather than mid-shot jumps.
4. Call `media.extract_keyframes` on unfamiliar video clips to verify visual subjects match the narrated sentence.

## 2. Storyline Architecture & Visual World Selection

Before placing clips, lock in **one Storyline Device × Camera Form** (`skill://director-core/references/direction-and-pacing.md`) and **one Visual World** (`skill://captions-and-gfx/references/motion-craft.md`) so the entire video feels directed rather than assembled from random templates:
- **SaaS / AI / Tech Demos**: `glass` (obsidian silk + frosted 3D glass panes + rack-focus DOF) or `product-ui` (dot-grid + interactive prompt/command bar + cursor triggers).
- **Editorial / Documentary / Myth-Busting**: `editorial-paper` (highlighter sweeps, strike-correct, receipts, stamps) or `monochrome` (architectural dot-grid + single accent focal point).
- **High-Energy Creator / Newsletter / Launch**: `kinetic-type` (giant 9–21% display slams + slot-machine word rolls) or `retro-print` (cream paper grain + on-twos 12fps motion).

## 3. Spine & Visual Density Contract (`A1` + `V1` + `V2` or Full-Canvas `G1`)

- **Audio Spine First (`A1`)**: Place voiceover on **`a1`** (never `a2`, which is reserved for ducked music). Trim dead air `> 0.35s` with `0.12s` breath pads while keeping a natural documentary cadence.
- **Zero Static Stretches (`<= 3.5s` Rule)**: Never hold a single unchanged visual on screen for more than **`3.5s`**. Change camera angle, cut to a fresh `V1`/`V2` clip, or trigger a living `G1` visual beat every `0.5–0.8s` (bar fills, sparkline draws, card lifts, specular sheen sweeps).
- **Mandatory Ken Burns on Stills**: Whenever placing a `MediaKind::Image` asset via `timeline.place_clip`, always set **`ken_burns: true`** so Solaris applies smooth centered zoom/pan motion in both WebGL2 preview and FFmpeg export.
- **Zero Visual Gaps**: Keep `V1` (or full-canvas `G1` in zero-asset mode) continuously covered from `0.0s` through the end of narration (`first visual <= 0.50s`, no uncovered gaps `> 0.40s`). Clamp `A2` music duration to the narration/visual end with a clean `0.8s` fade-out.
- **Beat-Synced Cutting**: Snap `V1`/`V2` cuts and `G1` entrances within **`±0.10s`** of `manifest.beats` or narration sentence starts.

## 4. Structured Graphics Hierarchy (`G1` + `T1`)

1. **Instrument, Not a Sentence**: Match each narrated beat to the exact visual archetype in `skill://captions-and-gfx/references/move-playbook.md` (and check `assets.search` first for Solaris's 5 starter cards when a lightweight overlay suffices):
   - **Hook (`0.0–2.5s`)**: Frame-0 ink immediately (no fade from black) + kinetic slam (`zoom` entrance) or `chapter-title-solar` paired with the strongest visual payoff.
   - **Proof / Metrics**: `kpi-tiles`, `bar-race`, `line-reveal`, `funnel-stages`, or `stat-card-solar` landed within `0.10s` of spoken stats—**only use real sourced numbers; hide counters until they start**.
   - **Mechanism / Steps**: `agent-pipeline`, `bento-reveal`, `explainer-build`, `terminal-run`, `code-diff`, or `progress-bar-solar` (top safe band `y: -0.30`).
   - **Comparison / Verdict**: `matrix-2x2`, `pros-cons`, `strike-correct`, or `verdict-card` stamped with semantic green `#2BD67B` / red `#FF5A5F`.
   - **End Card (`last 2.0–3.5s`)**: Mark/wordmark lands with weight, specular sweep runs once, holds cleanly to the end.
2. **Safe-Band Hygiene**: For full-canvas `G1` scenes, keep `Transform2D` at `(0, 0)` and extend ambient glow/grid across the full `100vh`. When `T1` captions exist, keep foreground cards/diagrams inside `6vh..62vh` on `9:16` (`6vh..72vh` on `16:9`) so captions never overlap; when **no `T1` captions exist**, balance foreground cleanly across `8vh..84vh` on `9:16` (`8vh..88vh` on `16:9`) so there is never a bottom-half black void. For floating `G1` overlays, offset via `timeline.set_transform` (`|gfx.y - cap.y| >= 0.08`).
3. **Captions Alignment**: Call `captions.generate_from_transcript` after `A1` is placed (never place fake captions when no narration transcript exists). For `16:9` explainers, prefer `minimal` or `mono_caps` at `3–4` words/burst unless high-energy `neon_glow`/`karaoke` is requested.

## 5. BANNED Faceless Explainer Defaults

- Placing voiceover audio onto `A2` (gets ducked as music!).
- Placing still images with `ken_burns: false` (reads as a dead PowerPoint slide).
- Leaving a static visual on screen for `> 3.5s`, or leaving a dead pitch-black bottom half on `9:16` programmatic scenes.
- Centered text on a plain gradient with no world, diagram, or kinetic rhythm.
- Mixing conflicting visual worlds across scenes, printing internal asset IDs as visible text, or inventing unsourced statistics.
- Letting `A2` music run past the final visual clip or drowning narration without `ducking_enabled: true`.
