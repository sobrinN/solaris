# Solaris Seekable HTML/CSS/GSAP GFX Contract

## Table of Contents

1. [What Is Bundled in the Sandbox](#1-what-is-bundled-in-the-sandbox)
2. [Brand CSS Variables & Template Props](#2-brand-css-variables--template-props)
3. [4 Deterministic Seek Hooks](#3-4-deterministic-seek-hooks)
4. [Hard Sandbox & Size Limits](#4-hard-sandbox--size-limits)
5. [Good vs Bad Animated GFX Examples](#5-good-vs-bad-animated-gfx-examples)

---

## 1. What Is Bundled in the Sandbox

Both the Studio iframe preview (`/api/v1/gfx/sandbox/{id}`) and headless Chromium export compile every GFX snippet with strict offline CSP (`default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src data:; font-src data:`) and automatically inject:

- **GSAP 3 UMD (`window.gsap`)**: Available globally inside inline `<script>` tags—zero CDN `<script src>` needed.
- **Bundled `Sora` variable font** (weights `100–800`) + any `.woff2`/`.ttf` custom fonts uploaded to the active **Brand Playbook** as offline `data:` `@font-face` rules.
- **Transparent full-bleed stage**: `html, body` render transparent over the video; center or position your inner card cleanly inside `.clip`.

---

## 2. Brand CSS Variables & Template Props

All 7 Brand Playbook design tokens are injected on `:root`:

| CSS Variable | Role | Default Fallback |
| --- | --- | --- |
| `--brand-bg` | Deep obsidian backdrop | `#080404` |
| `--brand-surface` | Elevated glass/card base | `#1A0E0E` |
| `--brand-ink` | Primary crisp text ink | `#F7F5EF` |
| `--brand-accent` | Primary brand energy accent | `#F05A00` |
| `--brand-accent-secondary` | Highlight rim / eyebrow / badge | `#F7B733` |
| `--brand-font-display` | Display headline font family | `Sora` |
| `--brand-font-body` | Secondary/body font family | `Sora` |

### `{{prop}}` Auto-Escaped Interpolation
Use `{{key}}` or `{{nested.key}}` anywhere in `html_source` when authoring reusable `SavedCodeAsset`s. Solaris automatically HTML-escapes interpolated values and deep-merges `gfx.place` `props` over `default_props`.

---

## 3. 4 Deterministic Seek Hooks

Solaris renders animations by seeking the document to exact local time `t` (in seconds, `0 <= t <= duration`) via `window.__solarisSeek(t)`—never real wall-clock playback. Use any of these **4 supported seek mechanisms**:

1. **GSAP Global Timeline (easiest)**: Build paused or standard tweens with `gsap.fromTo(...)` / `gsap.timeline()`; Solaris scrubs `window.gsap.globalTimeline.time(t, false)` automatically.
2. **Named GSAP Timeline**: Assign `window.__timelines = { main: tl };` (`tl.pause(0)`). Solaris seeks `tl.time(t, false)` deterministically.
3. **Custom Seek Callback**: Define `window.__solarisOnSeek = (t, duration) => { ... };` for custom counters, SVG stroke dashes, or bar widths derived purely from `t` and `duration`.
4. **Pure CSS `@keyframes`**: Solaris pauses and scrubs every WAAPI animation in `document.getAnimations()` to `currentTime = t * 1000`.

> **Animation frame budget**: Export captures multi-frame RGBA PNG sequences up to `180` frames per animated clip (`6.0s` at `30fps`). Keep custom animated GFX durations focused (`1.5–5.0s`) and give every graphic both a crisp **entrance (`0.35–0.55s`)** and a clean **exit (`0.20–0.35s`)**.

---

## 4. Hard Sandbox & Size Limits

Enforced across `gfx.lint_html`, `assets.save`, `gfx.place`, and `gfx.update_clip`:

- **Recommended root attributes**: Root container should declare `class="clip"`, `data-start`, and `data-duration` (`gfx.lint_html` warns if `data-start`/`data-duration` or `--brand-` CSS variables are missing).
- **Max size**: `128 KB` (`131,072` bytes) per HTML snippet (enforced on `assets.save`, `gfx.place`, and `gfx.update_clip`).
- **Asset ID format** (`assets.save`): `^[a-zA-Z0-9_-]{1,64}$`.
- **Allowed**: Inline `<style>`, inline `<script>`, inline `<svg>`, `data:` image URIs.
- **Strictly Forbidden** (will fail `gfx.lint_html`):
  - External tags/URLs: `<script src=...>`, external `http(s)://` in `<img src/srcset>`, `<link href>`, `<iframe src>`, SVG `<image href/xlink:href>`, CSS `@import`, non-`data:` CSS `url(...)`, `<meta http-equiv="refresh">`.
  - Embedded media/plugins: `<video>`, `<audio>`, `<source>`, `<object>`, `<embed>`.
  - Non-deterministic / network / nav JS APIs (in `<script>` or `on*=`): `Date.now()`, `performance.now()`, `new Date()`, `Math.random()`, `setTimeout()`, `setInterval()`, `requestAnimationFrame()`, `fetch()`, `XMLHttpRequest`, `import()`, `WebSocket`, `EventSource`, `navigator.sendBeacon`, `window.location`.

---

## 5. Good vs Bad Animated GFX Examples

### Good: Deterministic GSAP Metric Card (Passes Lint & Scrubs Frame-Accurately)

```html
<div class="clip" data-start="0" data-duration="3.0" style="display:grid;place-items:center;">
  <style>
    .card {
      padding: 16px 24px;
      border-radius: 18px;
      background: color-mix(in srgb, var(--brand-surface, #1A0E0E) 90%, transparent);
      border: 1px solid color-mix(in srgb, var(--brand-accent, #F05A00) 45%, transparent);
      color: var(--brand-ink, #F7F5EF);
      font-family: var(--brand-font-display, Sora), sans-serif;
      box-shadow: 0 20px 44px -18px rgba(0, 0, 0, 0.8);
      text-align: center;
    }
    .eyebrow {
      font-size: 11px;
      font-weight: 600;
      letter-spacing: 0.16em;
      text-transform: uppercase;
      color: var(--brand-accent-secondary, #F7B733);
    }
    .metric {
      margin-top: 4px;
      font-size: 36px;
      font-weight: 700;
      font-variant-numeric: tabular-nums;
      line-height: 1.1;
    }
  </style>
  <div class="card" id="metric-card">
    <div class="eyebrow">{{label}}</div>
    <div class="metric" id="metric-val">{{value}}</div>
  </div>
  <script>
    const tl = gsap.timeline({ paused: true });
    tl.fromTo("#metric-card", { y: 22, opacity: 0, scale: 0.94 }, { y: 0, opacity: 1, scale: 1, duration: 0.45, ease: "expo.out" }, 0);
    tl.to("#metric-card", { y: -12, opacity: 0, duration: 0.25, ease: "power3.in" }, 2.75);
    window.__timelines = { main: tl };
  </script>
</div>
```

### Bad: Non-Deterministic & External Dependencies (Fails `gfx.lint_html`)

```html
<!-- FAILS: external CDN script, missing class="clip" & data-start/data-duration, uses requestAnimationFrame & Date.now() -->
<script src="https://cdn.jsdelivr.net/npm/gsap@3/dist/gsap.min.js"></script>
<div id="box">Loading...</div>
<script>
  const start = Date.now();
  function tick() {
    document.getElementById("box").style.opacity = Math.min(1, (Date.now() - start) / 500);
    requestAnimationFrame(tick);
  }
  tick();
</script>
```
