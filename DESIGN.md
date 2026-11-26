# Solaris Design System (Solar Horizon NLE)

**Version:** 1.1.0  
**Inspired by:** Solar Horizon visual language, adapted for a standalone open-source AI video editor.

---

## 1. Visual Philosophy

1. **Media First & Aspect-Aware**: The Program Monitor, Source Monitor, and Broadcast Scopes carry primary visual weight while adapting to the active project's canvas format (`9:16`, `1:1`, or `16:9`) so portrait and square projects frame cleanly without wasted black sidebars. UI chrome recedes into obsidian darkness so color grading and video content remain color-accurate.
2. **Solar Restraint**: Heat pigments (`Vermelho Foco`, `Laranja Criação`, `Amarelo Ideias`) signal active playheads, focus rings, creative actions, and solar horizon atmosphere—never loud multi-hue UI clutter.
3. **Calm Durability**: Background media probing, proxy generation, and multi-agent swarm edits surface clear, compact status without exposing noisy infrastructure.
4. **Lightweight & Accessible**: Zero `framer-motion`; pure CSS compositor transforms (`transform`, `opacity`), `prefers-reduced-motion` parity, `focus-visible` rings, and WCAG AA contrast.

---

## 2. Color System

### Core Obsidian & Solar Pigments

| Pigment | Token | Hex | Role |
| --- | --- | --- | --- |
| **Void** | `--void` | `#080404` | Primary canvas, NLE backdrop, monitor wells |
| **Carbono** | `--carbono` | `#1A0E0E` | Elevated panels, glass `MediaComposer`, inspector cards |
| **Grafite** | `--grafite` | `#1F1F1F` | Secondary controls, neutral inputs, track lanes |
| **Areia** | `--areia` | `#F7F5EF` | Primary ink, display titles, active icons |
| **Vermelho Foco** | `--vermelho-foco` | `#C31818` | Destructive fills, recording bars, error indicators |
| **Laranja Criação** | `--laranja-criacao` | `#F05A00` | Playhead, focus rings, active indicators, primary energy |
| **Amarelo Ideias** | `--amarelo-ideias` | `#F7B733` | Solar rim highlights, gradient tips, success/ready badges |

### Semantic Rules
- `--primary`: `color-mix(in srgb, var(--laranja-criacao) 76%, var(--vermelho-foco))` paired with `Areia` (`#F7F5EF`) foreground.
- `--accent`: `color-mix(in srgb, var(--laranja-criacao) 16%, var(--carbono))` — **exclusively** a dark hover tint for menu rows and selectable items. Never use `--accent` as a bright focal text or border color.
- **Destructive Contrast Rule**: Never use `Vermelho Foco` as bare body text on Void/Carbono. Use `bg-vermelho-foco text-areia` or `border-vermelho-foco/40 bg-vermelho-foco/15 text-areia`.

### Curated 7-Track Semantic Accents (Timeline Clips Only)
Timeline clips remain low-glare obsidian (`#141010` / `#181313`) topped with a crisp **2px luminous semantic strip** so 7 tracks stay scannable without overpowering the Program Monitor:

| Track | Code | Role | Top Strip Hex |
| --- | --- | --- | --- |
| `T1` | `TEXT` | Word-synced captions | `#F7B733` (Amarelo Ideias) |
| `G1` | `GFX` | HTML/GSAP motion overlays | `#FB7185` (Coral Rose) |
| `V2` | `B-ROLL` | Insert footage & Ken Burns stills | `#38BDF8` (Ice Cyan) |
| `V1` | `A-ROLL` | Primary speaker / spine cuts | `#60A5FA` (Steel Blue) |
| `FX` | `GRADE` | Adjustment layer LUT & color | `#F05A00` (Laranja Criação) |
| `A1` | `VOICE` | Dialog / voiceover waveform | `#34D399` (Emerald) |
| `A2` | `MUSIC` | Beat track + ducking + SFX | `#A78BFA` (Soft Violet) |

---

## 3. Typography

Solaris uses a **single-family system** built on bundled **Sora** (`@fontsource-variable/sora`, weights `300–700`):
- **No external Google Fonts `<link>`** (works 100% offline in desktop mode and avoids CSP issues).
- **No monospace UI chrome**: Timecodes (`00:00:12:04`), LUFS (`-14.1 LUFS`), durations, and revision numbers use **Sora** with `font-variant-numeric: tabular-nums`.

| Role | Weight | Size | Tracking |
| --- | --- | --- | --- |
| Chat Empty Headline | `300` (gradient `400`) | `clamp(2rem, 4vw, 3.1rem)` | `-0.03em` |
| Panel / Inspector Title | `500`–`600` | `0.8125rem`–`0.9375rem` | `-0.015em` |
| Body & Controls | `400` | `0.8125rem`–`0.875rem` | default |
| Wordmark (`S O L A R I S`) | `500` uppercase | `0.75rem`–`0.875rem` | `0.30em` |
| Track Codes & Badges | `600` uppercase | `0.625rem`–`0.6875rem` | `0.12em` |

---

## 4. Radius, Surfaces & Signature Controls

- **Pre-Editor `MediaComposer`**: `rounded-[24px]` translucent Carbono glass (`bg-carbono/80 backdrop-blur-xl border border-areia/14`).
- **3-Zone Studio Header & `ProjectManagerPopover`**: Compact top bar with Solaris wordmark + project switcher (switch, rename, blank create, inline-confirmed delete), centered undo/redo + Swarm/Focus presence toggle, and right-aligned studio utilities (`Logs`, `AI`, `Brand`, `Export`).
- **Aspect-Aware Studio Grid**: On `lg+` viewports, `StudioWorkspace` measures the grid height and fits the Program Monitor column snugly to `9:16` or `1:1` canvases—redistributing reclaimed horizontal width to Director & Swarm, toggleable Broadcast Scopes, and the segmented Inspector—while preserving full widescreen hero width for `16:9` projects and Source Monitor mode.
- **Studio Panels & Cards**: `rounded-xl` / `rounded-2xl` (`12px–16px`) with hairline `border-areia/10`–`14`.
- **Pills & Primary CTAs**: `rounded-full` with `.btn-solar-outline` hairline gradient border (`Areia` → `Laranja Criação` → `Amarelo Ideias`).
- **Swarm Cursors**: Pure CSS `transform: translate3d(x, y, 0)` transitions (`cubic-bezier(0.16, 1, 0.3, 1)`), tagged with agent role pills (`director`, `razor`, `broll`, `grade`, `caps`, `gfx`, `mix`, `mcp`) and a 1-click **Swarm / Focus** visibility toggle.
