---
name: captions-and-gfx
description: Builds social safe-zone word-synced captions (T1) and studio-grade deterministic HTML/CSS/SVG/GSAP motion graphics (G1) in Solaris. Use when generating or styling captions, choosing starter GFX templates, picking spoken-cue visual moves from the 10-category move playbook, designing full-canvas programmatic scenes across 9 visual worlds, positioning overlays clear of TikTok/Reels UI, or authoring and linting custom GSAP animations. Not for audio mixing or color grading.
---

# Solaris Captions & Programmable GFX Craft

Solaris renders `G1` (GFX) under `T1` (Captions) in both the Studio sandboxed iframe preview and dual-path headless Chromium export (supporting both static PNGs and multi-frame RGBA animated sequences up to 180 frames per clip).

## Deep References (Read Before Custom Authoring)

- **Motion Physics, 9 Visual Worlds & Kinetic Choreography** (glass/studio/monochrome/retro-print/editorial recipes, role-based eases, 12 kinetic type entrances, 3D camera/DOF/SVG primitives): [references/motion-craft.md](references/motion-craft.md)
- **10-Category Move Playbook & Blueprints** (266 spoken-cue → visual-move archetypes + copy-pasteable lint-clean Solaris HTML/CSS/GSAP blueprints): [references/move-playbook.md](references/move-playbook.md)
- **Seekable HTML/CSS/GSAP Sandbox Contract** (4 seek hooks, bundled Sora/GSAP, `{{prop}}` escaping, hard lint rules & seek gotchas): [references/gfx-contract.md](references/gfx-contract.md)
- **Built-In Starter GFX Catalog** (exact `props_schema` of all 5 built-in assets): [references/starter-assets.md](references/starter-assets.md)

## 1. Word-Synced Captions (`T1`)

1. **With speech media**: Ensure the primary speech asset on `V1` or `A1` has a populated transcript (`media.get_manifest` → `media.transcribe` if empty), then call `captions.generate_from_transcript` (maps timestamps across `V1` and `A1`, splits across `>0.6s` silences, sets safe-zone `y: 0.20`, replaces prior auto-captions).
2. **Silent or Zero-Asset Videos (No Narration Transcript)**: Place NO captions. Captions must reflect real narration, so `T1` stays empty until narration audio exists and is transcribed. Never script caption text for a video that has no speech.
3. Keep `words_per_burst` between **`2` and `4`** (hard QA cap is `6` words) and choose the right `preset` (`neon_glow`, `karaoke`, `mono_caps`, or `minimal`) via `captions.set_style` or the placement tool.

## 2. Social Safe Zones & Anti-Collision Positioning

Enforced automatically by `project.run_qa`:
- **`9:16` Title-Safe Bounds**: Keep `T1` and floating `G1` inside **`x ∈ [-0.20, 0.20]`** and **`y ∈ [-0.35, 0.35]`** (avoids the bottom `22%` Reels/TikTok captions and right `15%` action rail).
- **`16:9` / `1:1` Safe Bounds**: `x ∈ [-0.35, 0.35]`, `y ∈ [-0.35, 0.35]`.
- **Mandatory Post-Place Positioning (Floating Overlays Only)**: `gfx.place` ALWAYS initializes `Transform2D` to `x: 0.0, y: 0.0, scale: 1.0`. Whenever a floating `G1` card/badge overlaps `T1` captions (`y: 0.20`), **immediately follow `gfx.place` with `timeline.set_transform`** on `track_id: "g1"` to place it at `y: -0.22` to `-0.28` (avoiding the center face band on talking heads). Maintain **`|gfx.y - cap.y| >= 0.08`** and **never stack two floating overlays within `4.0s`**.
- **Full-Canvas Programmatic `G1` Scenes**: Full-bleed scenes (`position:fixed;inset:0;width:100vw;height:100vh`) MUST stay at `(0, 0)` (never shift with `timeline.set_transform`). Extend layered ambient gradients/grids/glows across the full `100vh`. In `9:16`, when `T1` captions are active keep foreground cards/diagrams inside **`6vh..62vh`** so `64vh..76vh` stays clear for captions; when **no `T1` captions exist**, use a **3-zone CSS Grid (`grid-template-rows: auto 1fr auto; padding: 7vh 5vw 9vh;`)** spanning **`8vh..84vh`** (Header, Hero visual in `1fr`, Bottom callout/legend) so the bottom half is never a dead void. **NEVER** add unsolicited scene or step counters (`01 / 06`, `2/6`, `ETAPA 02`, `STEP 3`), fake bottom loading/progress bars, or repetitive identical card-deck footers unless the user explicitly requests them.

## 3. Search Library First, Author Studio-Grade Second

1. **Always call `assets.search` first**. Solaris ships 5 ready-made, brand-tokenized assets (`chapter-title-solar`, `stat-card-solar`, `comment-bubble-solar`, `lower-third-solar`, `progress-bar-solar`—see [references/starter-assets.md](references/starter-assets.md)).
2. **When authoring custom `inline_html`** (or in zero-asset programmatic mode):
   - Pick **one Visual World** (`references/motion-craft.md`) for the entire video and **one specific Move Archetype** per beat (`references/move-playbook.md` — an instrument, not a sentence).
   - Call **`gfx.place`** directly (1–2 scenes per turn to avoid output truncation; `gfx.place` and `assets.save` enforce `lint_gfx_html` server-side automatically—call `gfx.lint_html` only to pre-verify or debug a rejected snippet).
   - Save reusable custom designs with **`assets.save`** (`id` matching `[a-zA-Z0-9_-]{1,64}`).

## 4. Non-Negotiable Motion & Sandbox Craft Rules

- **Root & Offline Assets**: `<div class="clip" data-start="0" data-duration="<sec>">`. **GSAP 3 (`window.gsap`)**, variable **Sora**, and Brand `@font-face` fonts are **already injected offline**—never use `<script src>`, `<link>`, `@import`, external URLs, `<video>`, `<audio>`, or non-deterministic JS (`Date.now`, `Math.random`, `setTimeout`, `setInterval`, `requestAnimationFrame`, `fetch`).
- **Role-Based Easing & Living Mid-Scene Motion**: Arrive slow (`expo.out` `0.45–0.9s`), exit 40–50% shorter (`power3.in` `0.25–0.35s` with `immediateRender: false`), cameras `power2.inOut`, springs `back.out(1.25–2.4)` on **transforms ONLY** (NEVER tween `filter: blur()` on `back`/`elastic`!). Every scene `> 3.0s` must include continuous subtle GSAP mid-scene motion (orbital drift, `stroke-dashoffset` signal pulses, or pulsing alerts — never a frozen 3–6s hold) while keeping landed headlines dead still.
- **SVG & Diagram Anti-Collision**: Use CSS Flex/Grid HTML for multi-word labels, cards, and badges — never hand-place `<text>` over dense SVG node graphs. Keep short SVG node labels cleanly outside circle radii (`>= 14–20px` clearance), keep `>= 32px` inner padding inside enclosing shields/rings, never place dots over text, and never render empty hollow node circles without an icon or glowing core.
- **Typography & Shadow Hygiene**: Tracking `>= -0.025em`, descender/glow-safe `0.42em` mask floors, shadow-safe negative-inset wipes (`inset(-40% ...)`), zero em/en dashes (`—`/`–`), no pixel-snapping `width`/`top`/`left`/`letter-spacing` tweens, no pre-roll `"$0"` counters, no breathing/floating loops on landed titles.
- **Brand vs. Prompt Palette Precedence**: Use `--brand-*` tokens by default; when the user prompt specifies explicit hex colors, honor those exact hex colors directly while still using `var(--brand-font-display, Sora)` / `var(--brand-font-body, Sora)` for typography. Keep success/up/paid/check in semantic green `#2BD67B` and error/down/alert in semantic red `#FF5A5F`.
- **Hero Scale**: Hero focal elements fill `40%+` of the stage; body/SVG labels stay legible at `360px` phone width (`>= 14–16px` responsive).

## 5. Visual Verification

After placing captions and GFX, call `preview.capture_frame` at a timestamp where **both `T1` and `G1` are active**, verify zero overlap, crisp legibility at phone width, full `100vh` atmosphere (no dead pitch-black void), and brand color fidelity, then run `project.run_qa`.
