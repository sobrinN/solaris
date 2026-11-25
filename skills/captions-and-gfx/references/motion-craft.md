# Solaris Motion Craft, Visual Worlds & Kinetic Choreography

Read this before authoring any custom HTML/CSS/SVG/GSAP scene or motion-graphic overlay on `G1`. Every rule and numeric value here is adapted specifically for Solaris's offline deterministic sandbox (`window.gsap`, `--brand-*` CSS tokens, `window.__timelines["main"]`, zero external URLs, zero `Math.random`).

---

## 1. The 9 Visual Worlds (Choose One Cohesive Language Per Film)

Never mix incompatible visual languages across scenes. Pick **one** world for the entire video in `art_direction`, then feed the active Brand Playbook's `--brand-*` tokens into it:

| World ID | Best For | Ground & Material Numbers | Motion Signature |
| --- | --- | --- | --- |
| **`glass`** | SaaS/AI launches, data proof, channel intros | Dark ground (`--brand-bg`) + layered radial silk glows (`filter: blur(80px)`) + frosted glass panes (`rgba(255,255,255,.16→.04)`, `backdrop-filter: blur(22px) saturate(160%)`, `1px` top rim `45%`, `1px` hairline `12%`, drop `0 40px 90px -30px rgba(0,0,0,.45)`, accent glow `35%`) + `5%` SVG grain + `22%` vignette | One 3D `perspective: 1600px` camera rig, `expo.out` fly-ins from `z: -900`, specular `--sheen` sweep on landing, rack-focus DOF (`blur(7–9px)` off-focus) |
| **`studio`** | Bold product reveals, single-color brands | Glowing radial brand-color stage (`--brand-accent` / `--brand-bg`), subtle sunburst rays, glass hero object, designer cursor + dashed selection marquee | Snappy `back.out(1.6)` hero pop, cursor-triggered state changes, punchy camera pushes |
| **`monochrome`** | Founder pitches, agencies, technical authority | Neutral dark or warm-grey architectural dot-grid canvas (`radial-gradient`), giant soft typographic asterisks/crosshairs, dashed selection boxes, `--brand-accent` reserved strictly for ONE focal element | Crisp `power3.inOut` cursor drags, high-contrast typographic cuts, zero clutter |
| **`product-ui`** | Software demos, AI prompts, command bars | Dot-grid canvas, stroke-drawn SVG mark (`stroke-dashoffset`), interactive prompt/search bar typing + click trigger, dark disc scale wipe into UI showcase | Character-by-character typing (`28 cps`), cursor click (`scale: 0.82→1`) triggering instant next-frame response |
| **`logo-build`** | Brand intros, AI/software teasers | Dark obsidian stage, geometric particles/dots whipping into mark, 3D card carousel, prompt send, stroke resolution | Continuous camera flow, `expo.out` convergence from seeded scatter, zero hard cuts |
| **`retro-print`** | Newsletters, creator drops, square/social loops | Warm cream paper (`#F5EFE4`) + inline SVG `feTurbulence` grain, bold 2-tone sunburst, thick `2.5–3px` ink outlines, hard offset drop shadows (`4px 4px 0 #141210`), extruded display type | **On-twos stepped motion** (`steps(Math.max(1, Math.round(dur * 12)))`) at 12fps feel, crisp mechanical stamps |
| **`editorial-paper`** | Documentaries, myth-busting, evidence/receipts | Textured paper cards, highlighter sweeps (`scaleX: 0→1`), red strike-through corrections, rubber approval stamps, torn/notched SVG edges, crisp tabular mono | Tactile card flips, stamp slams (`scale: 1.4→1` + slight rotation settle), marker wipes |
| **`glass-folder`** | Vertical openers, creator/agency portfolios | Dark lit stage with a 3D tilted brand-colored glass container/folder; drawn cursor clicks it, folder turns 3/4 and fans real content cards out in a 3D arc | Cursor click → `back.out(1.4)` fan-out with `0.06s` stagger + specular sweep |
| **`kinetic-type`** | High-energy hooks, manifestos, rhythmic reels | Giant display typography (`9–21%` of frame height), bold color-field switches on the beat, slot-machine word rolls, counter stamps | `zoom` slams (`scale: 2.4, blur: 16px → 1`), descender-safe `mask` rises, zero filler cards |

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

## 2. Motion Physics & Role-Based Easing Rules

Never put the same ease on everything. Every motion role has a distinct physical curve:

| Role | Duration | Ease | Hard Rule |
| --- | --- | --- | --- |
| **Primary arrival** (hero card, headline) | `0.45–0.90s` | `expo.out` or `power4.out` | Arrive on a long, silky decelerating tail. |
| **Spring / stamp / badge pop** | `0.45–0.75s` | `back.out(1.25–2.4)` | **TRANSFORMS ONLY (`x/y/scale/rotation`)!** NEVER tween `filter: blur()` or `opacity` on `back` or `elastic`—overshoot below 0 produces invalid `blur(-0.5px)` and freezes stale blur! |
| **Exit / dismissal** | `0.22–0.38s` | `power3.in` or `power2.in` | **40–50% shorter** than entrance; accelerate cleanly out; always set `immediateRender: false`. |
| **Camera move** (`.rig` pan/push/orbit) | `0.55–0.75s` | `power2.inOut` or `power3.inOut` | Ease both ways; tween `.rig`, NEVER the root `.clip`. |
| **Continuous drift / ambient** | Full scene span | `sine.inOut` | Move the ambient field or whole rig as **ONE body**—never per-item wobble or breathing titles. |
| **Progress bar / rotation / scrub** | Span | `none` (linear) | Linear ONLY for continuous progress, counters, or rotation. |
| **Retro / paper on-twos** | Any `d` | `steps(Math.max(1, Math.round(d * 12)))` | Authentic 12fps editorial stop-motion feel. |

### Overlap, Stagger & Reading-Time Discipline
- **30–50% Overlap**: Start the next element when the previous is 30–50% done (fully sequential feels robotic; simultaneous feels cheap).
- **Staggers**: Letters `12–35ms`, words `60–90ms`, list items/cards `40–80ms`.
- **One Hero Move Per Shot**: Secondary elements travel `50–70%` as far and start `100–200ms` later.
- **Landed Title = Dead Still**: Once a headline lands, freeze it; shift motion to the diagram, specular sheen, camera drift, or next beat.
- **Reading-Time Budget**: Hold any line the viewer must read for **`0.5s + 0.3s × word_count`**; hold the final end card **`2.0–3.5s`**. Cut words before shrinking hold time.
- **No Pixel-Snapping Tweens**: NEVER tween `letter-spacing`, `width`, `top`, or `left` (they snap to whole pixels and judder). Use `x`, `y`, `scale`, `scaleX`, `scaleY`, `rotation`, `z`.
- **Typography Floors**: Tracking **never tighter than `-0.025em`** (tighter causes glyph overlap seams); readable body/SVG labels `>= 2.6%` of short side (`>= 28px` at `1080p`, or `>= 14–16px` responsive `clamp`); **no em dashes (`—`) or en dashes (`–`)** in on-screen copy (use a comma, colon, or period).
- **Semantic State Colors**: Success/up/check/paid states ALWAYS use semantic green (`#2BD67B`), negative/down/error states use semantic red (`#FF5A5F`)—never hijack `--brand-accent` for a green checkmark.

---

## 3. 12 Kinetic Typography Entrances & 8 Exits (Pure Solaris GSAP)

Vary entrances across scenes; never fade-and-slide-up every line. All snippets assume `const tl = gsap.timeline({ paused: true });` and `window.__timelines = { main: tl };`:

| Kind | Solaris GSAP Recipe | Best Use |
| --- | --- | --- |
| **`words`** | `tl.fromTo(words, { opacity: 0, filter: "blur(6px)" }, { opacity: 1, filter: "blur(0px)", duration: 0.16, stagger: 0.067, ease: "power2.out" }, t)` | Crisp editorial body/sub-lines |
| **`ink`** | `tl.fromTo(words, { opacity: 0.16 }, { opacity: 1, duration: 0.12, stagger: 0.085, ease: "none" }, t)` | Quiet supporting lines, subtitles |
| **`letters`** | `tl.fromTo(chars, { opacity: 0, filter: "blur(4px)" }, { opacity: 1, filter: "blur(0px)", duration: 0.14, stagger: 0.03, ease: "power2.out" }, t)` | Brand names, wordmarks |
| **`rise`** | Wrap inner span inside `.mask` (`0.42em` bottom floor): `tl.fromTo(inner, { yPercent: 150 }, { yPercent: 0, duration: 0.55, ease: "expo.out" }, t)` | Heavyweight headlines (no box-shadow clipping!) |
| **`mask`** | Wrap each `.w` word in `.mask`: `tl.fromTo(words, { yPercent: 160, filter: "blur(6px)" }, { yPercent: 0, filter: "blur(0px)", duration: 0.6, stagger: 0.06, ease: "expo.out" }, t)` | Signature multi-word reveals |
| **`zoom`** | `tl.fromTo(words, { opacity: 0, scale: 2.4, filter: "blur(16px)" }, { opacity: 1, scale: 1, filter: "blur(0px)", duration: 0.45, stagger: 0.09, ease: "expo.out" }, t)` | Frame-0 kinetic hook slams |
| **`split`** | Using `mulberry32` `r`: `tl.fromTo(chars, { opacity: 0, x: () => (r()-0.5)*260, y: () => (r()-0.5)*180, rotation: () => (r()-0.5)*40, filter: "blur(8px)" }, { opacity: 1, x: 0, y: 0, rotation: 0, filter: "blur(0px)", duration: 0.7, stagger: 0.012, ease: "expo.out" }, t)` | Dramatic title assembly |
| **`line`** | `tl.fromTo(el, { opacity: 0, filter: "blur(8px)" }, { opacity: 1, filter: "blur(0px)", duration: 0.2, ease: "power3.out" }, t)` | Short punchy callouts |
| **`scale`** | `tl.fromTo(el, { opacity: 0, scale: 1.18, filter: "blur(10px)" }, { opacity: 1, scale: 1, filter: "blur(0px)", duration: 0.5, ease: "expo.out" }, t)` | Hero word or big stat |
| **`wipe`** | `tl.fromTo(el, { clipPath: "inset(-40% 100% -40% -10%)" }, { clipPath: "inset(-40% -10% -40% -10%)", duration: 0.45, ease: "power3.inOut" }, t)` | Labels, UI bars (negative insets preserve shadows & descenders!) |
| **`type`** | `chars.forEach((c, i) => tl.set(c, { opacity: 1 }, t + i / 28))` + finite caret `tl.set(caret, ...)` toggles every `0.5s` | Prompts, terminal commands, AI inputs |
| **`slot`** | Stack option `<span>`s; toggle each with `tl.set(s, { opacity: 0 }, 0)` + `tl.fromTo(s, { opacity: 1, yPercent: -60, filter: "blur(4px)" }, { opacity: 1, yPercent: 0, filter: "blur(0px)", duration: 0.09, ease: "power2.out", immediateRender: false }, t + i*0.1)` | Cycling pill / "a word that opens worlds" |
| **`count`** | Hide until `t` (`tl.set(el, { opacity: 0 }, 0); tl.set(el, { opacity: 1 }, t)` — NEVER pre-print `$0`!), tween `{ v: from } -> { v: to }` with `power3.out` + `onUpdate`, then **stamp**: `tl.fromTo(el, { scale: 1.12 }, { scale: 1, duration: 0.35, ease: "expo.out", immediateRender: false }, t + dur)` | Sourced numbers, KPIs, revenue, timers |

**Exits** (always `immediateRender: false` so seeking backwards never corrupts early frames):
- `zoom` (`scale: 1.7, blur: 14px, opacity: 0, 0.25s power3.in`), `blur` (`blur: 8px, opacity: 0, 0.18s power2.in`), `words` (stagger `0.035s`), `up` (`yPercent: -40, blur: 6px, opacity: 0, 0.28s power3.in`), `down` (`yPercent: 150` into `.mask`, `0.30s power3.in`), `wipe` (`inset(-40% -10% -40% 100%)`), `scale` (`scale: 1.08, blur: 10px, opacity: 0, 0.25s`), `cut` (`tl.set(el, { opacity: 0 }, t)`).

---

## 4. World & Camera Choreography Primitives

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

// 6. SVG sparkline / path draw with riding dot + expanding pulse ring
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

// 8. Text fitter (measures actual text Range width, never over-shrinks flex/block boxes)
function fitText(el, maxW, minPx = 14) {
  let fs = parseFloat(getComputedStyle(el).fontSize);
  el.style.whiteSpace = "nowrap";
  const r = document.createRange();
  const w = () => { r.selectNodeContents(el); return r.getBoundingClientRect().width; };
  while (w() > maxW && fs > minPx) { fs -= 2; el.style.fontSize = fs + "px"; }
}
```
