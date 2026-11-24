---
name: captions-and-gfx
description: Social safe-zone captioning, reusable GFX asset lookup, and the deterministic seekable HTML/CSS/GSAP component contract for Solaris.
---

# Captions & Programmable GFX Craft

1. **Social UI Safe Zones (`9:16`)**:
   - Keep captions and overlays inside the **80% title-safe zone**: avoid the bottom `22%` (Reels/TikTok descriptions) and right `15%` (like/share buttons).
   - Keep caption bursts tight (**2–4 words per block**) via `captions.generate_from_transcript`, and customize visual style (`preset`, `font_family`, `highlight_color`) with `captions.set_style`.

2. **Search Library Before Authoring**:
   - **Always** call `assets.search` before creating custom `inline_html`. Reuse `chapter-title-solar`, `stat-card-solar`, or `comment-bubble-solar` whenever they fit.
   - Place GFX clips on `g1` with `gfx.place` (passing `asset_id` + `props`, or validated `inline_html`).
   - If you author a reusable graphic, validate it with `gfx.lint_html` and save it via `assets.save` with a clean `props_schema`.

3. **Seekable Component Contract**:
   - Root element must carry `class="clip"` plus `data-start` and `data-duration`.
   - Inline `<style>...</style>` blocks and `data:` image URIs are allowed.
   - Use Brand Playbook CSS variables: `var(--brand-bg)`, `var(--brand-surface)`, `var(--brand-ink)`, `var(--brand-accent)`, `var(--brand-accent-secondary)`, `var(--brand-font-display)`.
   - **Forbidden**:
     - External `<script src=...>`, external URLs in `<img>` (`src`/`srcset`), `<link href=...>`, `<iframe src=...>`, SVG `<image href=...>` / `xlink:href=...`, CSS `@import`, non-`data:` CSS `url(...)`, `<meta http-equiv="refresh">`, and embedded media/plugin tags (`<video>`, `<audio>`, `<source>`, `<object>`, `<embed>`).
     - Non-deterministic, network, or navigation JS APIs inside `<script>` blocks or inline `on*=` handlers: `Date.now()`, `performance.now()`, `new Date()`, `Math.random()`, `setTimeout()`, `setInterval()`, `requestAnimationFrame()`, `fetch()`, `XMLHttpRequest`, `import()`, `WebSocket`, `EventSource`, `navigator.sendBeacon`, and `window.location`. All motion must be driven deterministically by timeline time or CSS.
