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

**Prompt palette precedence**: Default to `--brand-*` color tokens when the user does not request specific colors. When `brand.lock_brand_palette` is `true`, strictly enforce the Brand Playbook's `--brand-*` palette even if the prompt mentions other colors. Otherwise (`lock_brand_palette: false`), when the user prompt explicitly specifies a custom color palette (e.g., `#0A0F1C` navy + `#00E5FF` cyan + `#FF4D4D` red), honor those exact hex colors directly while still binding typography to `var(--brand-font-display, Sora)` and `var(--brand-font-body, Sora)` so fonts stay brand-consistent and lint-clean.

### `{{prop}}` Auto-Escaped Interpolation
Use `{{key}}` or `{{nested.key}}` anywhere in `html_source` when authoring reusable `SavedCodeAsset`s. Solaris automatically HTML-escapes interpolated values and deep-merges `gfx.place` `props` over `default_props`.

---

## 3. 4 Deterministic Seek Hooks

Solaris renders animations by seeking the document to exact local time `t` (in seconds, `0 <= t <= duration`) via `window.__solarisSeek(t)`—never real wall-clock playback. Use any of these **4 supported seek mechanisms**:

1. **Named GSAP Timeline (recommended)**: Build `const tl = gsap.timeline({ paused: true }); ... window.__timelines = { main: tl };`. Solaris scrubs `tl.time(t, false)` deterministically.
2. **GSAP Global Timeline**: Standard `gsap.fromTo(...)` / `gsap.timeline()`; Solaris scrubs `window.gsap.globalTimeline.time(t, false)`.
3. **Custom Seek Callback**: Define `window.__solarisOnSeek = (t, duration) => { ... };` for pure math-driven counters, canvas/SVG updates, or waveform frames.
4. **Pure CSS `@keyframes`**: Solaris pauses and scrubs every WAAPI animation in `document.getAnimations()` to `currentTime = t * 1000`.

> **Animation frame budget**: Export captures multi-frame RGBA PNG sequences up to `180` frames per animated clip (`6.0s` at `30fps`). Keep animated GFX durations focused (`1.5–5.5s`). For floating overlays, enter from empty at `t=0` and exit cleanly in the last `0.25–0.38s`; for full-canvas programmatic scenes, complete the primary headline by `t <= 0.35s`, finish entering **ALL core hero, output, and footer/takeaway elements by `<= 45%` of `duration_sec` (max `~1.8s`)** so settled frames never show `opacity: 0` voids, and keep core elements visible to the scene cut.

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

### SVG, Diagram & Full-Stage Layout Contract (Zero Overlaps, Zero Bottom Voids)
1. **HTML for text, SVG for geometry**: Render multi-word labels, risk cards, callouts, and pills in CSS Flex/Grid containers — NEVER hand-place `<text x y>` or `position: absolute` labels over dense SVG node graphs (especially in Portuguese/Spanish where phrases run ~25% longer than English).
2. **Strict node, stroke & enclosure clearance**: When a short label (`<= 2` words) accompanies an SVG node, place it strictly outside the circle radius (`cy + r + 20` or `cy - r - 14`, never overlapping the ring border or connecting lines). Never place a cyan/accent dot on top of a text label or central panel title. Inside enclosing shapes (shields, frames, rings), keep `>= 32px` inner padding so labels never cross the outline.
3. **SVG rotation origin & symmetric centering**: NEVER animate `rotation` or `scale` on an SVG `<g>`/`<path>` without explicit `svgOrigin: "<cx> <cy>"` matching the `viewBox` center (or CSS `transform-box: fill-box; transform-origin: center;`) — rotating around SVG `(0,0)` swings globes and orbits off-center into pills and edges. Center hero SVG diagrams symmetrically at `(W/2, H/2)` inside their frame/shield, and keep radial orbit radii compact enough that wide labels (e.g. `Domínio WHOIS`) keep `>= 24px` margin from viewport edges.
4. **No empty placeholder nodes**: Every node circle must contain either a crisp linear SVG icon (`stroke-width: 1.8`) or a solid glowing core dot — never render empty hollow rings with floating orphan labels.
5. **Direct-child / class-scoped background SVGs & flow-positioned badges**: NEVER write a descendant selector like `.funnel svg { position: absolute; inset: 0; width: 100%; height: 100%; }` when nested icon `<svg>` elements exist inside that container — GSAP transforms make child cards containing blocks and stretch every icon `<svg>` directly over the card text. Always use `.funnel > svg` or an explicit class (`.funnel-bg`). Similarly, NEVER hang circular seals/badges off giant headlines with `position: absolute` and negative `right`/`top` offsets in 9:16 portrait — place badges in normal Flex/Grid flow, and keep decorative SVG rings behind foreground pills subtle (`opacity <= 0.35`, no accent dots touching pill edges).
6. **3-Zone Full-Stage CSS Grid Scaffold & Contrast Floor** (when no `T1` captions are active): Build full-canvas scenes with `display: grid; grid-template-rows: auto 1fr auto; row-gap: 2vh; align-items: center; padding: 7vh 5vw 9vh;` inside `position: fixed; inset: 0; width: 100vw; height: 100vh;` so Header, Hero Diagram (`1fr`), and Bottom Takeaway/Legend automatically balance across `8vh..84vh` with zero bottom-half void and `>= 18px` vertical clearance from `header`/`.sub`. NEVER force `height: 100%` stretch on tall `.hero` cards or apply `scale > 1.0` to a full-height card into adjacent rows. Keep even secondary footers/disclaimers `>= 13px` at `>= 0.70` ink opacity so nothing drops below `360px` phone readability.
7. **Continuous living motion (no dead holds `> 2.0s`)**: After the fast entrance (`<= 0.35s` headline, all core + footer elements settled by `<= 45%` of `duration_sec` / `<= 1.8s`), every scene `> 3.0s` MUST include subtle deterministic GSAP mid-scene motion through `data-duration` (e.g., centered `svgOrigin` orbital rotation, animated `stroke-dashoffset` data packets along graph edges, pulsing risk rings, or a `scale: 1.0 -> 1.03` push on compact diagrams) while keeping landed headlines dead still.

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
