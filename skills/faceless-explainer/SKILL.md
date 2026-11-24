---
name: faceless-explainer
description: Visual storytelling, Ken Burns still motion, beat-synced B-roll pacing, and animated callout cards for faceless explainers and product launches.
---

# Faceless Explainer Craft

1. **Visual Density**:
   - Never leave a static frame on screen for more than `3.5s`.
   - For still images (`MediaKind::Image`), place clips with `timeline.place_clip` setting `ken_burns: true` so FFmpeg and the stage apply subtle continuous zoom/pan motion.

2. **Beat-Synced Cuts**:
   - Inspect `beats` and `duration_sec` from `media.get_manifest`.
   - Snap `V1` / `V2` visual cuts and `G1` card entrances (`gfx.place`) within `0.10s` of musical beats or narration phrase boundaries.

3. **Structured Graphics Hierarchy**:
   - Open with a `chapter-title-solar` hook card (`0.0–2.5s`).
   - Reinforce key numbers or claims with `stat-card-solar` or `comment-bubble-solar` on `G1`, keeping `T1` captions clear of `G1` bounds.
   - Check `assets.search` before writing custom HTML/GSAP.
