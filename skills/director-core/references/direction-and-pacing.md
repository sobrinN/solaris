# Direction, Pacing & Anti-Slop Rules

**The bar is jaw-dropping, not technically valid.** A cut or programmatic video can pass every automated QA check and still feel flat, slow, or generic. The difference is never last-minute polish—it is making structural, visual, and rhythmic decisions up front as **exact numbers**.

## Table of Contents

1. [Pick a Storyline Device × Camera Form](#1-pick-a-storyline-device--camera-form)
2. [Turn Adjectives Into Numbers](#2-turn-adjectives-into-numbers)
3. [Beat-Grid Tempo & Energy Arc Math](#3-beat-grid-tempo--energy-arc-math)
4. [The 10-Point Jaw-Drop Checklist](#4-the-10-point-jaw-drop-checklist)
5. [Top Visual Moves Quick-Pick & Pre-Edit Shotlist Template](#5-top-visual-moves-quick-pick--pre-edit-shotlist-template)
6. [BANNED Amateur Defaults](#6-banned-amateur-defaults)

---

## 1. Pick a Storyline Device × Camera Form

Before placing a single clip, run the **3-Option `DEVICE × FORM` Discipline**: mentally test **3 structurally distinct `DEVICE × FORM` concepts** (never 3 variations of plain cuts), pick the strongest one, and name a **concrete physical medium rather than a vague mood** (e.g., *"smoked glass panes lit by one amber rim"* instead of *"dark tech vibe"*). A sequence of plain cuts between unrelated cards feels like a slideshow; a deliberate device × form feels directed.

| Storyline Device | What the Viewer Sees |
| --- | --- |
| **Result first** | The finished payoff, output, or boldest claim lands at `t=0.0s`, then the video pulls back to prove how it works. |
| **One object across the system** | A single concrete entity (a prompt, a recording, a dollar, an order) travels continuously through every stage. |
| **A word that opens worlds** | One rotating/slot-machine keyword or pill cycles through values, and each value opens its own visual scene. |
| **Life before and after** | The old broken workflow churns noisily, then cuts/transforms into the calm, sharp new state that takes over the frame. |
| **A number lands** | A real sourced metric is earned by context, counts up smoothly, stops, and stamps physically on the drop. |
| **Type carries the argument** | Giant kinetic typography (`9–21%` of frame height) and bold brand color fields cut cleanly on the beat. |

| Structural / Camera Form | How the Edit Moves |
| --- | --- |
| **One continuous camera** | Rigs push, dive, rack-focus, and travel from one spatial station to the next without jarring slide resets. |
| **Object-to-object chain** | Each shot ends on the exact element that opens the next (e.g., a button click or card expands into the next scene). |
| **Alternating punch-in spine** | Talking-head cuts alternate `1.00× ↔ 1.08–1.12×` with `V2` B-roll cutaways every `3.0–5.0s` and J/L audio bridges. |
| **Flight through stations** | 3–5 visual stations on a shared rail or architectural grid; each beat focuses one station while others sit back blurred. |
| **One frame that rebuilds** | A single persistent stage/composition rearranges its cards, charts, or layers cleanly on every bar. |

> **Solaris Timeline IR Transition Honesty**: Native `V1`/`V2` tracks execute **hard cuts + J/L audio bridges + `1.00× ↔ 1.08–1.12×` punch-ins** (there is no native cross-clip `xfade`/dissolve in the Timeline IR). Seamless dives, whips, disc wipes, and continuous camera rigs live *inside* multi-beat `G1` HTML/GSAP clips or via matched exit/entry states across adjacent `G1` clips.

---

## 2. Turn Adjectives Into Numbers

Translate every vague creative word into exact Solaris parameters before touching the timeline:

| Vague Adjective | Exact Solaris Translation |
| --- | --- |
| **"Hooky / Retention-first"** | First spoken word or visual payoff at `t=0.0s` (never black); `1.08×` punch-in (`timeline.set_transform`) or frame-0 kinetic slam / `chapter-title-solar` (`y: -0.22`), primary headline landed by `t <= 0.35s`. |
| **"Snappy / Fast-paced"** | Cut all silences `> 0.25s` leaving `0.12s` breath pads; B-roll cutaway every `3.0–4.5s` lasting `1.5–2.5s`; something new every beat (`0.5–0.6s`) in programmatic `G1`; never hold a static visual `> 3.5s`. |
| **"Premium / Glass"** | Layered brand radial silk ground + frosted glass (`rgba(255,255,255,.16→.04)`, `1px` top rim `45%`, `backdrop-filter: blur(22px)`, `perspective: 1600px`, specular `--sheen` sweep on landing, `4.5%` SVG grain, `22%` vignette). |
| **"Cinematic / Filmic"** | Log-to-Rec.709 base (`contrast: 1.16`, `saturation: 1.14`, slight negative shadow `lift`), brand `.cube` LUT at `lut_intensity: 0.72`, `temp: +0.05` warmth, zero blown highlights, DOF `blur(7–9px)` off-focus in `G1`. |
| **"Clean social typography"** | `2–4` words per burst on `T1` (`y: 0.20`), brand accent `highlight_color`, tracking `>= -0.025em`, floating `G1` offset `y: -0.22` (`>= 0.08` clearance), zero em/en dashes. |
| **"Broadcast audio mix"** | Voice normalized to `-14.0 LUFS`, music on `A2` at `-16 dB` base gain with `duck_target_db: -18.0`, `fade_in_sec: 0.15`, `fade_out_sec: 0.8`. |

---

## 3. Beat-Grid Tempo & Energy Arc Math

When `manifest.beats` is populated on `A2` music—or when pacing a programmatic `G1` video—lock scene boundaries and visual reveals to **whole bars and exact beat indices** (`t = offset + n * (60 / bpm)`, never accumulating rounded floats):

| Target Length | 120 BPM (bar `2.0s`) | 128 BPM (bar `1.875s`) | 100 BPM (bar `2.4s`) | 96 BPM (bar `2.5s`) |
| --- | --- | --- | --- | --- |
| **~10s** | 5 bars = `10.0s` | 5 bars = `9.4s` | 4 bars = `9.6s` | 4 bars = `10.0s` |
| **~15s** | 7 bars = `14.0s` | 8 bars = `15.0s` | 6 bars = `14.4s` | 6 bars = `15.0s` |
| **~20s** | 10 bars = `20.0s` | 10 bars = `18.75s` | 8 bars = `19.2s` | 8 bars = `20.0s` |
| **~30s** | 14 bars = `28.0s` | 16 bars = `30.0s` | 12 bars = `28.8s` | 12 bars = `30.0s` |

- **Vibe → BPM**: Launch/driving `118–128`, playful `110–120`, warm/editorial `90–105`, cinematic/minimal `90–110`.
- **4-Stage Energy Arc**:
  1. **Intro / Hook (`0–20%`, energy `0.35`)**: Immediate visual/verbal hook at frame 0.
  2. **Build (`20–45%`, energy `0.60`)**: Staggered proof, mechanism, or rising list beats.
  3. **Drop / Signature Moment (`45–80%`, energy `1.00`)**: Spend your boldness in ONE place—the biggest data reveal, visual transformation, or hero payoff.
  4. **Outro / End Card (`80–100%`, energy `0.45`)**: Mark lands cleanly, wordmark/CTA sweeps, holds `2.0–3.5s`.
- **Cut on the Beat, Change Layout on the Bar**: Snap `V1`/`V2` cuts and `G1` entrances within `±0.10s` of `manifest.beats` or spoken phrase onsets.
- **Motion Personality Lock (90/10 Rule)**: Lock **one** personality (`Playful`, `Premium`, `Corporate`, or `Energetic`) across the entire video so `90%` of cuts and entrances share one consistent physics language, reserving `10%` contrast strictly for the Drop/Signature Moment.
- **3-Layer Motion Energy**: Orchestrate every animated beat across three tiers—**Primary (`100%`)** hero subject move, **Secondary (`30–50%`, `+50–100ms` lag)** follow-through (`--sheen` sweep, shadow lift, badge settle, shockwave ring) on a different ease, and **Ambient (`10–20%`)** subtle field drift.
- **4-Act Micro-Story Split**: Shape key `G1` moves as **Anticipation** (quick wind-up/pre-settle) → **Action** (crisp hero arrival) → **Reaction** (`+50–100ms` secondary response) → **Resolution** (**`100–200ms` deliberate stillness** so the eye registers the landed state before the next beat).

---

## 4. The 10-Point Jaw-Drop Checklist

Verify every full edit or programmatic film against these 10 criteria:
1. **A world, not a page**: Extend layered brand light, subtle grid/grain, and cohesive materials across the full `100vh` canvas—never a flat void or dead black bottom half.
2. **Frame 0 is alive**: Never black, never a slow fade from `0`, never waiting for a logo to appear.
3. **Something new every beat**: Inside programmatic scenes, trigger a visual beat every `0.5–0.8s` (word lands, bar fills, sparkline draws, card lifts, light sweeps). Never leave a static stretch `> 3.5s`.
4. **Hero fills 40%+ of the stage**: Size hero typography (`9–21%` of frame height) and hero cards boldly. Small boxes floating in a huge empty frame read as amateur.
5. **Depth of field & hierarchy**: Push non-active elements back (`blur(7–9px)`, `opacity: 0.45–0.55`, negative `z`) so the viewer's eye locks instantly onto one focal point per beat.
6. **One signature moment on the drop**: Concentrate the boldest visual move on the climax instead of making every second shout at 10/10.
7. **Light moves, landed titles hold still**: Run one specular `--sheen` sweep across a card as it lands; keep landed headlines dead still (no breathing/floating loops).
8. **An instrument, not a sentence**: Show a counter stopping, a bar racing, a diff flipping, or a check stamping instead of a paragraph of text.
9. **Clean zero-ghosting handoffs**: Outgoing overlays exit cleanly (`opacity: 0`) before the next beat arrives; programmatic scenes stay crisp and contiguous with `0.0s` black gaps.
10. **The end card performs**: Mark/CTA lands with weight, sweeps once, and holds `2.0–3.5s`.

---

## 5. Top Visual Moves Quick-Pick & Pre-Edit Shotlist Template

Name exact archetype IDs from `skill://captions-and-gfx/references/move-playbook.md` inside `scene.visual_brief` so scene builders never fall back to generic slides:

| Script Beat | Exact Archetype ID(s) | Script Beat | Exact Archetype ID(s) |
| --- | --- | --- | --- |
| **Metrics / KPIs** | `kpi-tiles` | **Code / Diff** | `code-diff` |
| **Rankings / Leaderboards** | `bar-race` | **Myth-Busting / Error** | `strike-correct` |
| **Financial / Delta Bridge** | `waterfall-chart` | **Comparison / Judgment** | `verdict-card` |
| **Feature Grid / Overview** | `bento-reveal` | **Editorial Quote / Pull** | `headline-highlight` |
| **Product / UI Layers** | `exploded-ui` | **Numbered Step / Listicle** | `listicle-number` |
| **Spotlight / Focus Callout** | `spotlight-dim` | **Section / Chapter Opener** | `chapter-tag` |
| **AI Agent / Tool Pipeline** | `tool-call-trace` | **Alerts / Social Proof** | `notify-stack` |
| **CLI / Build / Benchmark** | `terminal-run` | **Live Growth / Follower Counter** | `follower-ticker-chip` |

Write this compact numbers-only plan before making timeline mutations (and pause for user confirmation during interactive sessions):

```markdown
### Shotlist Plan — <Project Name> (<Aspect>, <Target Duration>s)
- **Device × Form**: <e.g., Result-first × One continuous camera> | **Visual World**: <e.g., glass>
- **Brand & Look**: <brand_id> | Log recovery: <yes/no> | LUT: <path @ intensity> | Grade: contrast <x>, sat <y>, temp <z>
- **Audio Target**: Voice `-14.0 LUFS` (`A1`/`V1`) | Music `<media_id>` on `A2` at `-16 dB`, ducked to `-18 dB`
- **Captions (`T1`)**: preset `<preset>` (or "None — no narration"), `<2-4>` words/burst, `y: 0.20`, highlight `<hex>`

| Time | Track(s) | Shot / Visual Move | Motion / Framing | Overlay / Audio Cue |
| --- | --- | --- | --- | --- |
| `0.0–2.4s` | `V1`, `G1` | Hook: "<first words>" (`kinetic-slam`) | `scale: 1.08` punch-in | `chapter-title-solar` (`y: -0.24`) |
| `2.4–5.8s` | `V1` | Core claim (silences >0.25s trimmed) | `scale: 1.00` reset | `T1` captions (`y: 0.20`) |
| `5.8–8.2s` | `V2`, `G1` | B-roll `<media_id>` + `kpi-tiles` | `ken_burns: true` / `gain_db: -99` | `stat-card-solar` (`y: -0.22`) |
| `8.2–12.0s` | `V1`, `G1` | Proof + CTA (`verdict-card`) | `scale: 1.10` emphasis | Music fade out `0.8s` |
```

---

## 6. BANNED Amateur Defaults

Reject and fix any of these before delivering. **Growing BANNED List Discipline**: whenever QA catches one of these defaults, add it to the repair pass's explicit banned list so the fix cannot jump to a cousin default (e.g., swapping a fake progress bar for a `01 / 05` step pill):

- **Empty frame 0 / slow fade-in**: Starting with black frames, dead air, or a `>0.5s` lead gap before the hook lands.
- **Centered text on a plain gradient**: A type-only scene with no kinetic rhythm, lighting, or architectural world.
- **Unedited dead air or static holds `>3.5s`**: Leaving pauses `>0.35s` in a talking-head spine, or holding an unchanged still (`ken_burns: false`) or static graphic with no new beat.
- **Linear spatial motion or opacity-only major state changes**: Moving elements on `linear` / `none` ease (reads robotic) or swapping major cards/states with `opacity` alone without paired transform, clip, or secondary reaction.
- **Everything fading and sliding up on the same ease**: Robotic uniform motion with no role-based easing (`expo.out` vs `power3.in` vs `back.out`).
- **Breathing, floating, or pulsing loops on titles or UI**: Landed text must stay dead still; drift only the ambient field or whole camera rig as one body.
- **Generic stock-icon slop or emoji**: Never use emoji or cheap clip-art where crisp inline SVG geometry or brand marks belong.
- **Caption/GFX collisions or social chrome burial**: Placing floating `G1` at `y: 0.0` or `y: 0.20` over `T1`/face, or dropping copy into the bottom `22%` (`y > 0.35`) or right `15%` (`x > 0.20`) on `9:16`.
- **Tracking tighter than `-0.025em` or clipped descenders/shadows**: Causes letter seams and box-sliced drop shadows.
- **Em dashes (`—`) or en dashes (`–`) in on-screen copy**: Use commas, colons, or short sentences.
- **100% LUT crush / raw Log gray / music drowning voice**: Shipping flat Log footage, slamming `lut_intensity: 1.0`, or leaving `A2` music un-ducked.
- **Invented numbers or placeholder filler**: Putting unsourced stats (`"300% FASTER"`), pre-roll `"$0"`, or internal asset IDs (`"progress-bar-solar"`) on screen.
- **Unsolicited scene counters & fake progress/loading bars**: Never stamp `01 / 06`, `2/5`, or useless bottom loading/progress bars on every scene, and never trap every beat inside the exact same top-pill + dark-box + 3-row checklist slide template unless the user explicitly asks for a step counter or progress bar.
- **Accent text on an accent ground**: Red text on red glow disappears—keep high contrast.
