# Solaris Seekable HTML/CSS/GSAP GFX Contract

## Table of Contents

1. [What Is Bundled in the Sandbox](#1-what-is-bundled-in-the-sandbox)
2. [Brand CSS Variables & Template Props](#2-brand-css-variables--template-props)
3. [4 Deterministic Seek Hooks](#3-4-deterministic-seek-hooks)
4. [Hard Sandbox, Lint & Rendering Gotchas](#4-hard-sandbox-lint--rendering-gotchas)
5. [Good vs Bad Animated GFX Examples](#5-good-vs-bad-animated-gfx-examples)

---

## 1. What Is Bundled in the Sandbox

Both the Studio iframe preview (`/api/v1/gfx/sandbox/{id}`) and headless Chromium export compile every GFX snippet with strict offline CSP (`default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src data:; font-src data:`) and automatically inject:

- **GSAP 3 UMD (`window.gsap`)**: Available globally inside inline `<script>` tags—zero CDN `<script src>` needed.
- **Bundled `Sora` variable font** (weights `100–800`) + any `.woff2`/`.ttf` custom fonts uploaded to the active **Brand Playbook** as offline `data:` `@font-face` rules.
- **Transparent full-bleed stage**: `html, body` render transparent over the video; floating overlays stay transparent around their inner card, while full-canvas programmatic scenes fill `position: fixed; inset: 0; width: 100vw; height: 100vh`.

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

**Semantic state tokens**: Keep success/up/paid/check states in semantic green (`#2BD67B`) and error/down/alert states in semantic red (`#FF5A5F`) rather than reusing `--brand-accent`.

### `{{prop}}` Auto-Escaped Interpolation
Use `{{key}}` or `{{nested.key}}` anywhere in `html_source` when authoring reusable `SavedCodeAsset`s. Solaris automatically HTML-escapes interpolated values and deep-merges `gfx.place` `props` over `default_props`.

---

## 3. 4 Deterministic Seek Hooks

Solaris renders animations by seeking the document to exact local time `t` (in seconds, `0 <= t <= duration`) via `window.__solarisSeek(t)`—never real wall-clock playback. Use any of these **4 supported seek mechanisms**:

1. **Named GSAP Timeline (recommended)**: Build `const tl = gsap.timeline({ paused: true }); ... window.__timelines = { main: tl };`. Solaris scrubs `tl.time(t, false)` deterministically.
2. **GSAP Global Timeline**: Standard `gsap.fromTo(...)` / `gsap.timeline()`; Solaris scrubs `window.gsap.globalTimeline.time(t, false)`.
3. **Custom Seek Callback**: Define `window.__solarisOnSeek = (t, duration) => { ... };` for pure math-driven counters, canvas/SVG updates, or waveform frames.
4. **Pure CSS `@keyframes`**: Solaris pauses and scrubs every WAAPI animation in `document.getAnimations()` to `currentTime = t * 1000`.

> **Animation frame budget**: Export captures multi-frame RGBA PNG sequences up to `180` frames per animated clip (`6.0s` at `30fps`). Keep animated GFX durations focused (`1.5–5.5s`). For floating overlays, enter from empty at `t=0` and exit cleanly in the last `0.25–0.38s`; for full-canvas programmatic scenes, complete the primary headline by `t <= 0.35s` and keep core elements visible to the scene cut.

---

## 4. Hard Sandbox, Lint & Rendering Gotchas

Enforced across `gfx.lint_html`, `assets.save`, `gfx.place`, and `gfx.update_clip`:

- **Root attributes**: Always declare `<div class="clip" data-start="0" data-duration="<sec>">` and use `--brand-*` CSS variables.
- **Max size**: `128 KB` (`131,072` bytes) per HTML snippet (keep programmatic scenes crisp at `2.5–6 KB` so tool payloads never truncate).
- **Asset ID format** (`assets.save`): `^[a-zA-Z0-9_-]{1,64}$`.
- **Allowed**: Inline `<style>`, inline `<script>`, inline `<svg>`, `data:` image URIs (including inline SVG `feTurbulence` grain).
- **Strictly Forbidden** (fails `gfx.lint_html`):
  - External tags/URLs: `<script src=...>`, external `http(s)://` in `<img src/srcset>`, `<link href>`, `<iframe src>`, SVG `<image href/xlink:href>`, CSS `@import`, non-`data:` CSS `url(...)`, `<meta http-equiv="refresh">`.
  - Embedded media/plugins: `<video>`, `<audio>`, `<source>`, `<object>`, `<embed>`.
  - Non-deterministic / network / nav JS APIs (in `<script>` or `on*=`): `Date.now()`, `performance.now()`, `new Date()`, `Math.random()`, `setTimeout()`, `setInterval()`, `requestAnimationFrame()`, `fetch()`, `XMLHttpRequest`, `import()`, `WebSocket`, `EventSource`, `navigator.sendBeacon`, `window.location`.

### Critical Seek & Render Gotchas (Prevent Subtle Visual Bugs)
1. **Always set `immediateRender: false`** on exit tweens and any repeated `fromTo` targeting the same element (e.g. a blinking dot or second sheen sweep), or seeking backwards to `t=0` shows the final state.
2. **NEVER tween `filter: blur()` on `back` or `elastic` eases**: Overshoot dips below `0px`, producing invalid `blur(-0.5px)` that leaves a frozen blur on the frame. Put `back.out` on transforms (`x/y/scale/rotation`) and `power2.out` / `expo.out` on `blur` and `opacity`.
3. **NEVER tween `letter-spacing`, `width`, `top`, or `left`**: They snap to integer pixels and judder. Tween `x`, `y`, `scale`, `scaleX`, `scaleY`, `rotation`, and `z` instead.
4. **Never clip shadowed text at the border box**: A `clip-path: inset(0 100% 0 0)` slices drop shadows and descenders (`g`, `y`) flat. Use shadow-safe negative insets (`inset(-40% 100% -40% -10%)`) for wipes and a `0.42em` bottom pad/negative-margin floor for rise masks.
5. **Tracking floor `-0.025em`**: Tighter letter-spacing overlaps glyphs and renders hairline seams through letters.
6. **Never pre-print `$0` on counters**: Hide a counter until its count starts, then count and stamp (`scale: 1.12 -> 1`).
7. **Deterministic randomness only**: Replace `Math.random()` with a 4-line `mulberry32` seeded PRNG (see `references/motion-craft.md`).
8. **Responsive scaling**: Use `vw`, `vh`, `vmin`, `clamp(...)`, `<svg viewBox>`, or `u = Math.min(window.innerWidth, window.innerHeight) / 1080` so scenes render identically in compact Studio preview and full `1080×1920` / `1920×1080` export.

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
      box-shadow: inset 0 1px 0 rgba(255,255,255,0.35), inset 0 0 0 1px color-mix(in srgb, var(--brand-accent, #F05A00) 45%, transparent), 0 20px 44px -18px rgba(0,0,0,0.8);
      color: var(--brand-ink, #F7F5EF);
      font-family: var(--brand-font-display, Sora), sans-serif;
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
      font-weight: 800;
      letter-spacing: -0.025em;
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
    tl.to("#metric-card", { y: -12, opacity: 0, duration: 0.25, ease: "power3.in", immediateRender: false }, 2.75);
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
