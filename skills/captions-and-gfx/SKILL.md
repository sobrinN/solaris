---
name: captions-and-gfx
description: Builds social safe-zone word-synced captions (T1) and deterministic seekable HTML/CSS/GSAP motion-graphic overlays (G1) in Solaris. Use when generating or styling captions, choosing starter GFX templates, positioning overlays clear of TikTok/Reels UI, or authoring and linting custom GSAP title cards, stat callouts, lower thirds, and progress bars. Not for audio mixing or color grading.
---

# Solaris Captions & Programmable GFX Craft

Solaris renders `G1` (GFX) under `T1` (Captions) in both the Studio sandboxed iframe preview and dual-path headless Chromium export (supporting both static PNGs and multi-frame RGBA animated sequences).

## Deep References (Read Before Custom Authoring)

- Full HTML/CSS/GSAP runtime contract, 4 seek hooks, bundled Sora/GSAP, and 👍/👎 code patterns: [references/gfx-contract.md](references/gfx-contract.md)
- Exact catalog & `props_schema` of all 5 built-in starter GFX assets: [references/starter-assets.md](references/starter-assets.md)

## 1. Word-Synced Captions (`T1`)

1. Ensure the primary speech asset on `V1` or `A1` has a populated transcript (`media.get_manifest` → `media.transcribe` if empty).
2. Call `captions.generate_from_transcript`:
   - Maps timestamps automatically across both `V1` and `A1`, splits across `>0.6s` silence gaps, sets safe-zone `y: 0.20`, and **replaces prior auto-captions** cleanly (no stacked duplicates).
   - Keep `words_per_burst` between **`2` and `4`** (hard QA cap is `6` words).
3. Choose the right `preset` (or restyle all `T1` clips via `captions.set_style` with `font_family` and `highlight_color` from `brand.get_playbook`):
   - `neon_glow`: high-energy dark pill with glowing active word (default for creator `9:16` shorts).
   - `karaoke`: filled highlight pill behind the spoken word (bold retention reels).
   - `mono_caps`: tight uppercase broadcast strip (tech demos, punchy headlines).
   - `minimal`: restrained editorial subtitle (16:9 explainers, documentary).

## 2. Social Safe Zones & Anti-Collision Positioning

Enforced automatically by `project.run_qa`:
- **`9:16` Title-Safe Bounds**: Keep `T1` and `G1` inside **`x ∈ [-0.20, 0.20]`** and **`y ∈ [-0.35, 0.35]`** (avoids the bottom `22%` Reels/TikTok captions and right `15%` action rail).
- **`16:9` / `1:1` Safe Bounds**: `x ∈ [-0.35, 0.35]`, `y ∈ [-0.35, 0.35]`.
- **Mandatory Post-Place Positioning**: `gfx.place` ALWAYS initializes `Transform2D` to `x: 0.0, y: 0.0, scale: 1.0`. Whenever a `G1` clip overlaps `T1` captions (which default to `y: 0.20`), **immediately follow `gfx.place` with `timeline.set_transform`** on `track_id: "g1"` to place it at `y: -0.22` to `-0.28` (or `y: 0.0` only when no speaker face sits at center).
- Maintain **`|gfx.y - cap.y| >= 0.08`** whenever `G1` and `T1` overlap in time.

## 3. Search Library First, Author Second

1. **Always call `assets.search` first**. Solaris ships 5 ready-made, brand-tokenized assets (`chapter-title-solar`, `stat-card-solar`, `comment-bubble-solar`, `lower-third-solar`, `progress-bar-solar`—see [references/starter-assets.md](references/starter-assets.md)).
2. Place a library asset with `gfx.place(project_id, clip_id, label, start_sec, duration_sec, asset_id, props)`—partial `props` merge cleanly over `default_props`.
3. Only author custom `inline_html` when no library asset fits the creative beat. Read [references/gfx-contract.md](references/gfx-contract.md) before writing HTML, validate with **`gfx.lint_html`**, place via `gfx.place`, and save reusable designs with **`assets.save`** (`id` matching `[a-zA-Z0-9_-]{1,64}`).

## 4. Seekable Sandbox Quick Rules (Summary)

- Root container **must** have `class="clip" data-start="0" data-duration="<sec>"`.
- **GSAP 3 (`window.gsap`)**, variable **Sora**, and uploaded Brand `@font-face` fonts are **already bundled and injected offline**—never load CDNs.
- Use brand CSS variables (`--brand-bg`, `--brand-surface`, `--brand-ink`, `--brand-accent`, `--brand-accent-secondary`, `--brand-font-display`, `--brand-font-body`) and `{{prop}}` placeholders.
- Drive motion deterministically via CSS `@keyframes`, `gsap.timeline()`, `window.__timelines["main"]`, or `window.__solarisOnSeek = (t, duration) => { ... }`.
- **Forbidden**: external URLs (`http://`, `<script src>`, `<link>`, `@import`, non-`data:` `url()`), media/plugin tags (`<video>`, `<audio>`, `<object>`), and non-deterministic/network JS (`Date.now`, `performance.now`, `new Date`, `Math.random`, `setTimeout`, `setInterval`, `requestAnimationFrame`, `fetch`, `XMLHttpRequest`, `WebSocket`, `import()`).

## 5. Visual Verification

After placing captions and GFX, call `preview.capture_frame` at a timestamp where **both `T1` and `G1` are active**, verify zero overlap, crisp legibility at phone width, and brand color fidelity, then run `project.run_qa`.
