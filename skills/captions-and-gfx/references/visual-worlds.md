# Solaris 9 Visual Worlds Lookbook (Pure Web + GSAP 3)

Pick **one** cohesive world per film in `art_direction`, then feed `--brand-*` tokens into it. Every recipe below is 100% offline-safe (no WebGL, no Three.js, no external JPG/PNGs, zero `Math.random`).

---

## 1. 9-World Selection Matrix

| World ID | Best For | Ground & Palette Discipline | Motion Signature & BPM | Typography Scale Rule | Signature Pure-Web Technique |
| --- | --- | --- | --- | --- | --- |
| **`glass`** | SaaS/AI launches, data proof, channel intros | Dark `--brand-bg` + silk light ribbons + frosted `.glass` (`4.5%` grain, `22%` vignette) | 3D `perspective: 1600px` `expo.out` fly-ins, `--sheen` sweep, rack-focus DOF (`120–128 BPM`) | `700–800` tight display (`-0.025em`), `tabular-nums` KPIs | CSS/Canvas2D silk ribbons + 4-layer `.glass` stack + 3×3 turning logo wall |
| **`studio`** | Bold product reveals, single-color brands | Lit `color-mix()` accent radial ground or `.is-mono` `#ECECEC` `28px` dot grid | Springy `back.out(1.3–2.0)` / `back.in(1.6)`, never stepped (`100–120 BPM`) | High-contrast inverted pill keyword (`#fff` / `--st-deep`) | 24-ray `repeating-conic-gradient` sunburst + measured dashed selection marquee + lens orb |
| **`glass-folder`** | Vertical openers, creator/agency portfolios | Moody dark radial + `blur(36px)` horizontal accent band + overhead light cone | Cursor click → `3/4` folder turn + 5-card Z-fan `back.out(1.2–1.35)` (`105–115 BPM`) | Dissolving ghost headline behind sharp white foreground title | Low-blur `7px` tinted frosted glass front (content behind stays legible) |
| **`monochrome`** | Founder pitches, agencies, technical authority | Neutral `#F1F1F1→#D9D9D9` or `#121214` + `30px` dot grid; `--brand-accent` on ONE item | Crisp `0.32s power3.out` cursor drag locked 1:1 to selection box (`96–104 BPM`) | Binary extreme scale: tiny `400` uppercase kicker vs fat `900` headline | Dual rotating polar soft-asterisks + angled V-curtains |
| **`product-ui`** | Software demos, AI prompts, command bars | Flat `#E8E8E8` (or obsidian) `30px` dot grid, zero grain, zero vignette | `28 cps` typing + cursor click ring (`0.4→1.3`) + instant UI response (`116–124 BPM`) | Crisp UI sans (`500–600`), monospace-free `tabular-nums` | SVG inner-rim bevel filter + frame-0-safe `L*0.82` stroke draw + tilted dark disc wipe |
| **`logo-build`** | Brand intros, AI/software teasers | Warm paper `#F4F3EF` + 3 drifting radial pools (or obsidian stage) | `onUpdate` parametric dots→arc→whip→orb→carousel (`108–116 BPM`) | Clean centered wordmark + single-line `.meta` on active card only | Velocity-smeared whip (`scaleX = 1 + a*0.25`) + deterministic 3D carousel + zero-stray-cap wipe |
| **`retro-print`** | Newsletters, creator drops, social loops | Cream `#FCE9CB` dual-`feTurbulence` speckled paper + zero blur shadows | Strict **on-twos `12fps`** (`steps(Math.max(1, Math.round(d*12)))`, `100–108 BPM`) | 9-layer hard-extruded `900` uppercase + `4px`-stroked subtitle | Live `feDisplacementMap` (`scale: 5`) SVG ink-boil cycling `seed` every `1/12s` |
| **`editorial-paper`** | Documentaries, myth-busting, receipts | `#F3F3F1` `38px` graph grid or warm newsprint + multiplied `feTurbulence` grain | `12fps` `twos()` easing on cutouts while `#cam` pushes smoothly (`96–112 BPM`) | Editorial serif/sans mix + felt-tip corrections & marker highlights | Procedural `.rim`/`.ink` torn-paper polygons + conflict-free `.boil` CSS vars |
| **`kinetic-type`** | High-energy hooks, manifestos, reels | Bold beat-locked color-field switches (`--brand-bg` ↔ `--brand-accent`) | Beat-locked `zoom` slams, slot-machine rolls, waveform slams (`120–132 BPM`) | Giant `9–21%` height display (`-0.025em` floor, `lh: 0.9–0.96`) | Pure CSS 3D extruded ordinals (`K=18..22` stacked `translateZ` spans + glass `.face`) |

---

## 2. Concrete Pure-Web Recipes

### 1. `glass` — Dark Silk & Frosted 3D Stage (`120–128 BPM`)
- **Ground (No WebGL)**: `--brand-bg` + multi-layered CSS radial silk ribbons (`.ambient i { filter: blur(90px); opacity: .42 }`) or `<canvas>` 2D cubic-bezier silk ribbons driven by `window.__solarisOnSeek(t)`. Overlay `4.5%` SVG `feTurbulence` `.grain` + `22%` `.vignette`.
- **4-Layer `.glass` Stack**: `background: linear-gradient(150deg, rgba(255,255,255,.16), rgba(255,255,255,.04))`, `backdrop-filter: blur(22px) saturate(160%)`, `box-shadow: inset 0 1px 0 rgba(255,255,255,.45), inset 0 0 0 1px rgba(255,255,255,.12), 0 40px 90px -30px rgba(0,0,0,.45), 0 0 80px -10px color-mix(in srgb, var(--brand-accent) 35%, transparent)`.
- **`.glass.is-light` Variant**: `linear-gradient(150deg, rgba(255,255,255,.78), rgba(255,255,255,.52))`, top rim `rgba(255,255,255,.95)`, hairline `rgba(0,0,0,.06)`.
- **Signature Choreography**: `--sheen` screen sweep (`-40% → 140%`) on landing; rack-focus DOF (`z: +80, blur(0)` vs `z: -260, blur(7–9px), opacity: 0.45–0.55`); 1-to-6 3D arc burst; 3×3 turning logo wall (`rotationY: 0 → 180` staggered `0.04s`).

### 2. `studio` — Lit Accent Stage & Refractive Lens (`100–120 BPM`)
- **Ground**: Radial lit stage via `color-mix()`: `--st-hi: color-mix(in srgb, var(--brand-accent) 62%, #fff)`, `--st-mid: var(--brand-accent)`, `--st-deep: color-mix(in srgb, var(--brand-accent) 22%, #050506)`. `.is-mono` variant: `#ECECEC` + `28px` radial dot grid.
- **Sunburst & Blobs**: Rotating 24-ray `repeating-conic-gradient` masked radially (`mask-image: radial-gradient(circle, #000 18%, transparent 70%)`) + harmonic flower blobs (`border-radius` / SVG).
- **Selection Marquee & Glass**: Pre-split measured dashed marquee snaps a keyword to inverted high-contrast `#fff` / `--st-deep`; `blur(14px)` glass folder; `blur(3px) saturate(180%)` refractive lens orb with a top-left specular ellipse. Springy `back.out(1.3–2.0)` / `back.in(1.6)` transforms only—never stepped.

### 3. `glass-folder` — Moody Cone & Low-Blur Frosted Fan (`105–115 BPM`)
- **Ground & Light**: Dark studio radial + horizontal blurred accent light band (`filter: blur(36px)`) + overhead conic light beam.
- **Ghost + Sharp Title**: Dissolving ghost headline (`background: linear-gradient(180deg, color-mix(in srgb, var(--brand-accent) 65%, #fff), color-mix(in srgb, var(--brand-accent) 85%, transparent) 45%, transparent 92%); -webkit-background-clip: text; color: transparent`) overlapped by sharp `--brand-ink` foreground title.
- **Low-Blur Front & Fan**: Use **low-blur `backdrop-filter: blur(7px)`** tinted glass front so faces/cards behind stay legible (never `22px` fog). Chunky `30px`-stroked 3D glass SVG cursor clicks (`scale: 0.62 → 0.54 → 0.62`), turning the folder `3/4` and fanning 5 stacked Z-cards on `back.out(1.2–1.35)`.

### 4. `monochrome` — Polar Asterisks & Binary Scale (`96–104 BPM`)
- **Ground**: Strict neutral `#F1F1F1 → #D9D9D9` (or `#121214`) + `30px` dot grid + local masked `34px` grid. Reserve `--brand-accent` strictly for **ONE** focal element.
- **Elements**: Two rotating polar soft-asterisks; designer black/white-stroked cursor locked 1:1 with a dashed selection-box drag (`0.32s power3.out`); angled V-curtains for wipes.
- **Typography**: Binary extreme scale—tiny `400` uppercase kicker vs fat `900` headline in stepped off-center pairs.

### 5. `product-ui` — Flat Dot-Grid, Bevel & Frame-0 Stroke (`116–124 BPM`)
- **Ground**: Flat `#E8E8E8` (or obsidian `#0B0B0D`) `30px` dot grid—**zero grain, zero vignette**.
- **Inner-Rim Bevel & Frame-0 Logo**: SVG bevel filter (`feGaussianBlur` + `feOffset` + `feComposite` `arithmetic`) on dark marks. Frame-0-safe stroke draw: initialize `strokeDashoffset: L * 0.82` at `t = 0` (ink visible immediately) → `0` → fill fade → `back.out(2)` scale pulse.
- **Interaction**: Prompt/search bar typing (`28 cps`) + click ring expansion (`scale: 0.4 → 1.3`); giant tilted dark disc atomic scene wipe (`scale: 0 → 26`).

### 6. `logo-build` — Warm Pools, Whip & 3D Carousel (`108–116 BPM`)
- **Ground**: Warm paper `#F4F3EF` + 3 drifting soft radial pools (accent + warm gold `rgba(255,214,140,.30)`) or dark obsidian stage.
- **Parametric Choreography**: `onUpdate` parametric dots → arc → velocity-smeared whip (`scaleX = 1 + a * 0.25`) into mark → glowing orb → deterministic 3D carousel `pose(k) = { x: k * 360, z: -Math.abs(k) * 320, rotationY: -k * 30 }` (only active center card fades `.meta` in).
- **Zero-Stray-Cap Wipe**: `stroke-linecap="round"` stroke wipe with `opacity: 0 → 1 → 0` keyed to exact draw start/end frames so no dot lingers at `t = 0` or `t = end`.

### 7. `retro-print` — Dual-Turbulence Paper & Live Ink-Boil (`100–108 BPM`)
- **Ground**: Cream `#FCE9CB` mottled + dark-fleck speckled dual inline SVG `feTurbulence` paper (zero external JPGs) + 40-ray two-red `repeating-conic-gradient` sunburst.
- **Ink & Extrusion**: 9-layer hard-extruded `900` uppercase title + `4px`-stroked overprinted subtitle; zero blur shadows (offset solid `#160c0a` duplicate paths); `shape-rendering="crispEdges"` pixel screen.
- **Live Ink-Boil & On-Twos**: SVG `feDisplacementMap` (`scale: 5`, filter region `x="-5%" y="-5%" width="110%" height="110%"`) cycling `feTurbulence` `seed` every `1/12s` via `__solarisOnSeek`. Strict **on-twos `12fps`** (`steps(Math.max(1, Math.round(d * 12)))`).

### 8. `editorial-paper` — Procedural Cutouts & Conflict-Free Boil (`96–112 BPM`)
- **Ground**: `#F3F3F1` `38px` graph grid OR warm newsprint + multiplied inline SVG `feTurbulence` grain (zero JPGs).
- **Procedural Cutouts**: Dual-clip `.rim` / `.ink` torn-paper `clip-path: polygon(...)` with uneven `#FBFAF6` white margins + translucent CSS tape strips (`linear-gradient` + jagged ends).
- **Conflict-Free Boil & Camera**: `.boil { translate: var(--bx, 0px) var(--by, 0px); rotate: var(--br, 0deg); }` cycling every `1/6s` without fighting GSAP `transform`. Quantize cutouts to `12fps` `twos()` while `#cam` pushes smoothly. Chisel-marker `mix-blend-mode: multiply` highlighter sweeps & curved felt-tip SVG strike-throughs.

### 9. `kinetic-type` — Beat-Locked Fields & CSS 3D Ordinals (`120–132 BPM`)
- **Typography**: Giant `9–21%` height display type (`-0.025em` tracking floor, `line-height: 0.9–0.96`), beat-locked color-field switches, slot-machine word rolls, audio-waveform emphasis slams.
- **Pure CSS 3D Extruded Ordinals (Zero WebGL)**: Stack `K = 18..22` child `<span>` layers inside `transform-style: preserve-3d`, each offset by `translateZ(-i * dz)` with progressively darkened `color-mix()` ink, topped by a glass `.face` with `--sheen` sweep.

---

## 3. 3-Mode Surface Adaptation

Every world adapts cleanly to the slot's visual role:
1. **Full-Canvas Scene (`G1` programmatic)**: Full `100vw × 100vh` world + grain/vignette (if world uses them) + 3-zone CSS Grid (`auto 1fr auto`).
2. **Transparent Talking-Head Overlay (`G1` floating over `V1`)**: Zero full-screen grain, zero vignette, zero opaque ground; high-opacity card body (`88–92%` `--brand-surface`); keep overlays in face-safe bounds (`y: -0.22..-0.28` or lower-third/corner).
3. **Chroma-Key Safe**: Solid opaque fills only, `backdrop-filter: none`, hard edges, zero soft outer glows or semi-transparent shadows.
