# Solaris Motion Craft, Choreography & Kinetic Physics

Read this before authoring any custom HTML/CSS/SVG/GSAP scene or motion-graphic overlay on `G1`. Every rule and numeric value here is adapted specifically for Solaris's offline deterministic sandbox (`window.gsap`, `--brand-*` CSS tokens, `window.__timelines["main"]`, zero external URLs, zero `Math.random`).

---

## 1. Visual Worlds & Core CSS Layer

Pick **one** cohesive world for the entire video in `art_direction` (`glass`, `studio`, `glass-folder`, `monochrome`, `product-ui`, `logo-build`, `retro-print`, `editorial-paper`, or `kinetic-type`) and feed `--brand-*` tokens into it. See [`visual-worlds.md`](./visual-worlds.md) for the full 9-world selection matrix, pure-web recipes, `color-mix()` grounds, and 3-mode surface adaptation.

### Core World CSS Layer (Drop Into Any Full-Canvas Scene)

```css
/* Deterministic 3D Stage + Frosted Glass + Specular Sweep + Film Grain */
.world { position: fixed; inset: 0; perspective: 1600px; perspective-origin: 50% 46%; overflow: hidden; background: var(--brand-bg, #080404); }
.rig { position: absolute; inset: 0; transform-style: preserve-3d; will-change: transform; }
.ambient { position: absolute; inset: -18%; pointer-events: none; }
.ambient i { position: absolute; border-radius: 50%; filter: blur(90px); opacity: 0.42;
  background: radial-gradient(circle, var(--c, var(--brand-accent, #F05A00)) 0%, transparent 68%); }
.glass {
  border-radius: 28px; overflow: hidden;
  background: linear-gradient(150deg, rgba(255,255,255,0.16) 0%, rgba(255,255,255,0.04) 100%);
  -webkit-backdrop-filter: blur(22px) saturate(160%); backdrop-filter: blur(22px) saturate(160%);
  box-shadow: inset 0 1px 0 rgba(255,255,255,0.45), inset 0 0 0 1px rgba(255,255,255,0.12),
    0 40px 90px -30px rgba(0,0,0,0.45), 0 0 80px -10px color-mix(in srgb, var(--brand-accent, #F05A00) 35%, transparent);
}
/* Specular light sweep: tween --sheen from -40% to 140% ONCE when card lands */
.glass::after {
  content: ""; position: absolute; inset: -2px; pointer-events: none; border-radius: inherit; mix-blend-mode: screen;
  background: linear-gradient(105deg, transparent calc(var(--sheen, -40%) - 18%), rgba(255,255,255,0.28) var(--sheen, -40%), transparent calc(var(--sheen, -40%) + 18%));
}
.lit { color: var(--brand-accent-secondary, #F7B733); text-shadow: 0 0 28px color-mix(in srgb, var(--brand-accent, #F05A00) 55%, transparent); }
/* Descender & glow-safe mask floor (NEVER clip at 0% bottom!) */
.mask { display: inline-block; padding: 0.12em 0.2em 0.42em; margin: -0.12em -0.2em -0.42em; clip-path: inset(-30% -10% 0% -10%); }
/* Deterministic static film grain (zero JS, passes offline CSP) */
.grain { position: fixed; inset: -16px; pointer-events: none; opacity: 0.045; mix-blend-mode: overlay; z-index: 90;
  background-image: url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='220' height='220'><filter id='n'><feTurbulence type='fractalNoise' baseFrequency='0.9' numOctaves='2' seed='4'/></filter><rect width='220' height='220' filter='url(%23n)'/></svg>"); }
.vignette { position: fixed; inset: 0; pointer-events: none; z-index: 89;
  background: radial-gradient(120% 90% at 50% 45%, transparent 55%, rgba(0,0,0,0.28) 100%); }
```

---

## 2. Motion Personality, Physics & Choreography Budgets

### Motion Personality Archetypes (90/10 Brand Rule)
Lock **ONE** archetype for `90%+` of a project; reserve the remaining `10%` for intentional climax/warning breaks:

| Archetype | Micro / Standard / Macro | Primary Ease | Overshoot & Squash-Stretch |
| --- | --- | --- | --- |
| **`Playful`** | `150 / 250 / 400ms` | `back.out(1.6–2.2)` | `10–20%` overshoot; squash-stretch `[1.06, 0.94]` |
| **`Premium`** | `350 / 500 / 800ms` | `power3.out` / `cubic-bezier(0.4, 0, 0.2, 1)` | `0%` overshoot; **never** squash-stretch |
| **`Corporate`** | `200 / 300 / 450ms` | `power2.out` / `cubic-bezier(0.2, 0, 0, 1)` | `0–3%` overshoot; crisp settle |
| **`Energetic`** | `100 / 180 / 300ms` | `expo.out` | `15–30%` impact slam + recoil |

### Role-Based Curves, 3-Layer Energy & 4-Act Micro-Story
- **3-Layer Motion Energy**: `Primary (100%)` hero arrival → `Secondary (30–50%)` `+50–100ms` delayed with a distinct ease → `Ambient (10–20%)` continuous drift/shimmer.
- **4-Act Micro-Story**: Anticipation (`10–15%`) → Action (`35–50%`) → Reaction (`10–20%`) → Resolution (`25–30%` + `100–200ms` stillness).
- **Hard Choreography Budgets**:
  - **1/3 Distance Rule**: Travel `<= 1/3` of viewport or element span with a `10–20px` midpoint arc.
  - **1/3 Density Rule**: Max `1/3` of elements (or `2–3` items) moving simultaneously.
  - **Total Stagger `< 500ms`**: `20–40ms` micro, `50–100ms` standard, `30–60ms` wave.
  - **Counter-Motion (`15–30%`)**: When a hero enters left→right, nudge its sibling/container `15–30%` opposite.
  - **Depth-Through-Speed Parallax**: FG `1.0x`, Mid `0.5x`, BG `0.2x`, Deep `0.1x`.
  - **Simplicity Threshold**: `2` animated properties is the sweet spot; **NEVER** use opacity-only fades for major state changes.
  - **Material Modifiers**: `Rigid` (`1.2x` duration, `0%` overshoot), `Paper` (`1.0x`, `3–5%`), `Glass` (`0.9x`, `0%`), `Elastic` (`0.8x`, `15–25%`), `Fluid` (`1.5x`, `5%`).
  - **Emotion → Path & Color Semantics**: Success transitions TO `#2BD67B`; Error flashes `#FF5A5F` + `±10–15px` horizontal shake with `0%` overshoot.
- **Safety & Reading Discipline**:
  - **NEVER** tween `filter: blur()` or `opacity` on `back` or `elastic` eases (negative overshoot freezes stale blur). Put `back.out` on transforms only; use `power2.out` / `expo.out` on `blur`/`opacity`.
  - **NEVER** tween `letter-spacing`, `width`, `top`, or `left`. Exits are `40–50%` shorter (`0.22–0.38s` `power3.in`) with `immediateRender: false`.
  - **Reading-Time Budget**: `0.5s + 0.3s × word_count`; hold end cards `2.0–3.5s`. Landed titles stay dead still.
  - **Typography Floors**: Tracking `>= -0.025em`; readable body/SVG labels `>= 2.6%` of short side; **no em dashes (`—`) or en dashes (`–`)** in on-screen copy.

---

## 3. 15 Kinetic Typography Entrances & 8 Exits (Pure Solaris GSAP)

Vary entrances across scenes. All snippets assume `const tl = gsap.timeline({ paused: true });` and `window.__timelines = { main: tl };`:

| Kind | Solaris GSAP Recipe | Best Use |
| --- | --- | --- |
| **`present`** | `tl.set(el, { opacity: 1 }, 0)` (already visible at `t = 0`, optionally paired with subtle `scale: 1.03 → 1`) | Frame-0 hooks & logos |
| **`pop`** | `tl.fromTo(words, { scale: 0.2, y: 50 }, { scale: 1, y: 0, duration: 0.42, stagger: 0.06, ease: "back.out(2)" }, t)` + `tl.fromTo(words, { opacity: 0 }, { opacity: 1, duration: 0.22, stagger: 0.06, ease: "power2.out" }, t)` | Playful/energetic word pops |
| **`words`** | `tl.fromTo(words, { opacity: 0, filter: "blur(6px)" }, { opacity: 1, filter: "blur(0px)", duration: 0.16, stagger: 0.067, ease: "power2.out" }, t)` | Crisp editorial body/sub-lines |
| **`ink`** | `tl.fromTo(words, { opacity: 0.16 }, { opacity: 1, duration: 0.12, stagger: 0.085, ease: "none" }, t)` | Quiet supporting lines |
| **`letters`** | `tl.fromTo(chars, { opacity: 0, filter: "blur(4px)" }, { opacity: 1, filter: "blur(0px)", duration: 0.14, stagger: 0.03, ease: "power2.out" }, t)` | Brand names, wordmarks |
| **`rise`** | Wrap inner span inside `.mask` (`0.42em` bottom floor): `tl.fromTo(inner, { yPercent: 150 }, { yPercent: 0, duration: 0.55, ease: "expo.out" }, t)` | Heavyweight headlines |
| **`mask`** | Wrap each `.w` in `.mask`: `tl.fromTo(words, { yPercent: 160, filter: "blur(6px)" }, { yPercent: 0, filter: "blur(0px)", duration: 0.6, stagger: 0.06, ease: "expo.out" }, t)` | Multi-word reveals |
| **`zoom`** | `tl.fromTo(words, { opacity: 0, scale: 2.4, filter: "blur(16px)" }, { opacity: 1, scale: 1, filter: "blur(0px)", duration: 0.45, stagger: 0.09, ease: "expo.out" }, t)` | Kinetic hook slams |
| **`split`** | Using `mulberry32` `r`: `tl.fromTo(chars, { opacity: 0, x: () => (r()-0.5)*260, y: () => (r()-0.5)*180, rotation: () => (r()-0.5)*40, filter: "blur(8px)" }, { opacity: 1, x: 0, y: 0, rotation: 0, filter: "blur(0px)", duration: 0.7, stagger: 0.012, ease: "expo.out" }, t)` | Dramatic title assembly |
| **`line`** | `tl.fromTo(el, { opacity: 0, filter: "blur(8px)" }, { opacity: 1, filter: "blur(0px)", duration: 0.2, ease: "power3.out" }, t)` | Short punchy callouts |
| **`scale`** | `tl.fromTo(el, { opacity: 0, scale: 1.18, filter: "blur(10px)" }, { opacity: 1, scale: 1, filter: "blur(0px)", duration: 0.5, ease: "expo.out" }, t)` | Hero word or big stat |
| **`wipe`** | `tl.fromTo(el, { clipPath: "inset(-40% 100% -40% -10%)" }, { clipPath: "inset(-40% -10% -40% -10%)", duration: 0.45, ease: "power3.inOut" }, t)` | Labels, UI bars |
| **`type`** | `chars.forEach((c, i) => tl.set(c, { opacity: 1 }, t + i / 28))` + finite caret toggle every `0.5s` | Prompts, terminal inputs |
| **`slot`** | Stack option `<span>`s; `tl.set(s, { opacity: 0 }, 0)` + `tl.fromTo(s, { opacity: 1, yPercent: -60, filter: "blur(4px)" }, { opacity: 1, yPercent: 0, filter: "blur(0px)", duration: 0.09, ease: "power2.out", immediateRender: false }, t + i*0.1)` | Cycling pill options |
| **`count`** | Hide until `t` (`tl.set(el, { opacity: 0 }, 0); tl.set(el, { opacity: 1 }, t)`), tween `{ v: from } -> { v: to }` with `power3.out` + `onUpdate`, then **stamp** (`scale: 1.12 -> 1`, `0.35s expo.out`) | Sourced KPIs, timers |

**8 Exits** (always `immediateRender: false`): `zoom` (`scale: 1.7, blur: 14px, opacity: 0, 0.25s power3.in`), `blur` (`blur: 8px, opacity: 0, 0.18s power2.in`), `words` (stagger `0.035s`), `up` (`yPercent: -40, blur: 6px, opacity: 0, 0.28s power3.in`), `down` (`yPercent: 150` into `.mask`, `0.30s power3.in`), `wipe` (`inset(-40% -10% -40% 100%)`), `scale` (`scale: 1.08, blur: 10px, opacity: 0, 0.25s`), `cut` (`tl.set(el, { opacity: 0 }, t)`).

---

## 4. 14 World & Camera Choreography Primitives

```js
// 1. Deterministic Seeded PRNG (NEVER use Math.random!)
const rand = (seed) => () => {
  seed |= 0; seed = (seed + 0x6d2b79f5) | 0;
  let r = Math.imul(seed ^ (seed >>> 15), 1 | seed);
  r = (r + Math.imul(r ^ (r >>> 7), 61 | r)) ^ r;
  return ((r ^ (r >>> 14)) >>> 0) / 4294967296;
};

// 2. Arrive from 3D depth (transforms on back/expo; blur on power2.out separately if using back.out!)
tl.fromTo("#card", { z: -900, y: 120, rotationX: 20, opacity: 0, filter: "blur(14px)" },
  { z: 0, y: 0, rotationX: 0, opacity: 1, filter: "blur(0px)", duration: 0.85, ease: "expo.out" }, 0.1);

// 3. Specular light sweep across a .glass card ONCE after it lands
tl.fromTo("#card", { "--sheen": "-40%" }, { "--sheen": "140%", duration: 0.85, ease: "power2.inOut", immediateRender: false }, 0.55);

// 4. Rack-focus depth of field across 3 plates in a 3D arc
const focusPlate = (plates, on, at) => plates.forEach((p, i) => tl.to(p, {
  z: i === on ? 80 : -240, rotationY: [18, 0, -18][i],
  filter: i === on ? "blur(0px)" : "blur(8px)", opacity: i === on ? 1 : 0.5,
  duration: 0.45, ease: "power3.out"
}, at));

// 5. Drawn SVG cursor moving to target and clicking ON THE EXACT FRAME of state change
tl.to("#cursor", { x: tx, y: ty, duration: 0.45, ease: "power3.inOut" }, clickAt - 0.45);
tl.fromTo("#cursor", { scale: 0.82 }, { scale: 1, duration: 0.16, ease: "power2.out", immediateRender: false }, clickAt);

// 6. SVG sparkline / path draw with riding dot (use style.strokeDashoffset ONLY, never GSAP attr)
const L = pathEl.getTotalLength();
pathEl.style.strokeDasharray = L; pathEl.style.strokeDashoffset = L;
const prog = { p: 0 };
tl.to(prog, { p: 1, duration: 1.2, ease: "power3.inOut", onUpdate: () => {
  const len = L * prog.p; pathEl.style.strokeDashoffset = L - len;
  const pt = pathEl.getPointAtLength(len);
  dotEl.setAttribute("cx", pt.x); dotEl.setAttribute("cy", pt.y);
}}, 0.4);

// 7. Beat punch on the camera rig (NEVER on a title)
tl.fromTo("#rig", { scale: 1.04 }, { scale: 1, duration: 0.32, ease: "expo.out", immediateRender: false }, beatTime);

// 8. Centered SVG orbital rotation (ALWAYS pass svgOrigin matching viewBox center!)
tl.to("#globe-rings", { rotation: 24, svgOrigin: "270 270", duration: 4.0, ease: "none" }, 0);

// 9. Continuous SVG edge data-packet pulse (total span <= data-duration; immediateRender: false)
tl.fromTo(".edge-pulse", { strokeDashoffset: 180 }, { strokeDashoffset: 0, duration: 1.4, ease: "none", repeat: 2, immediateRender: false }, 0.8);

// 10. Measure-Before-Tween Text Fitter (call BEFORE gsap.from/fromTo on flat un-transformed DOM!)
function fitText(el, maxW, minPx = 14) {
  let fs = parseFloat(getComputedStyle(el).fontSize);
  el.style.whiteSpace = "nowrap";
  const r = document.createRange();
  const w = () => { r.selectNodeContents(el); return r.getBoundingClientRect().width; };
  while (w() > maxW && fs > minPx) { fs -= 2; el.style.fontSize = fs + "px"; }
}

// 11. Single-Clock Deterministic Driver (parametric arcs, Canvas2D silk, boil seeds)
const clock = { t: 0 };
tl.to(clock, { t: D, duration: D, ease: "none", onUpdate: () => drive(clock.t) }, 0);

// 12. Signature Impact Climax Stack (slam -> recoil -> shockwave -> rig kick -> sheen -> dim)
tl.fromTo("#hero", { scale: 1.6, y: -40 }, { scale: 0.96, y: 0, duration: 0.14, ease: "power4.in" }, tHit);
tl.to("#hero", { scale: 1, duration: 0.32, ease: "back.out(3)", immediateRender: false }, tHit + 0.14);
tl.fromTo("#shockwave", { scale: 1, opacity: 0.75 }, { scale: 2.6, opacity: 0, duration: 0.45, ease: "expo.out", immediateRender: false }, tHit + 0.14);
tl.fromTo("#rig", { scale: 1.025 }, { scale: 1, duration: 0.28, ease: "expo.out", immediateRender: false }, tHit + 0.14);
tl.fromTo("#hero", { "--sheen": "-40%" }, { "--sheen": "140%", duration: 0.65, ease: "power2.inOut", immediateRender: false }, tHit + 0.18);
tl.to(".sibling", { opacity: 0.55, duration: 0.25, ease: "power2.out" }, tHit + 0.14);

// 13. 12fps On-Twos + Conflict-Free CSS Boil (CSS: .boil { translate: var(--bx,0px) var(--by,0px); rotate: var(--br,0deg); })
const twos = (d, ease = "power2.out") => { const e = gsap.parseEase(ease), n = Math.max(1, Math.round(d * 12)); return (p) => Math.ceil(e(p) * n) / n; };
// In drive(t): const step = Math.floor(t * 6); el.style.setProperty("--bx", ((step % 3) - 1) * 1.2 + "px");

// 14. Directional SVG Motion Blur + Zero-Stray-Cap Stroke Wipe + Running Conic Rim
// Motion blur: <filter id="mb"><feGaussianBlur id="mb-std" stdDeviation="0 0"/></filter> -> set stdDeviation=`${vx} 0`
// Zero-stray-cap wipe: stroke-linecap="round"; opacity 0 -> 1 at drawStart, 1 -> 0 at drawEnd
// Running conic rim: @property --ang { syntax: "<angle>"; inherits: false; initial-value: 0deg; } + conic-gradient(from var(--ang), ...)
```
