# Direction, Pacing & Anti-Slop Rules

## Table of Contents

1. [Turn Adjectives Into Numbers](#1-turn-adjectives-into-numbers)
2. [Pre-Edit Shotlist & Cut Plan Template](#2-pre-edit-shotlist--cut-plan-template)
3. [Hook & Visual Momentum Patterns](#3-hook--visual-momentum-patterns)
4. [BANNED Amateur Defaults](#4-banned-amateur-defaults)

---

## 1. Turn Adjectives Into Numbers

Vague creative words ("cinematic", "punchy", "clean", "dynamic") produce generic, forgettable edits. Translate every creative adjective into exact Solaris parameters before touching the timeline:

| Vague Adjective | Exact Solaris Translation |
| --- | --- |
| **"Hooky / Retention-first"** | First spoken word or visual payoff at `t=0.0s`; `1.08×` punch-in (`timeline.set_transform`) or `chapter-title-solar` (`0.0–2.2s` at `y: -0.22`). |
| **"Snappy / Fast-paced"** | Cut all silences `> 0.25s` leaving `0.12s` breath pads; B-roll cutaway every `3.0–4.5s` lasting `1.5–2.5s`; never leave a static visual `> 3.5s`. |
| **"Cinematic / Filmic"** | Log-to-Rec.709 base (`contrast: 1.16`, `saturation: 1.14`, slight negative shadow `lift`), brand `.cube` LUT at `lut_intensity: 0.72`, `temp: +0.05` warmth, zero blown highlights. |
| **"Clean social typography"** | `2–4` words per burst on `T1`, `y: 0.20`, `scale: 1.0`, `opacity: 1.0`, brand accent `highlight_color`, GFX offset at `y: -0.22` (`>= 0.08` clearance). |
| **"Broadcast audio mix"** | Voice normalized to `-14.0 LUFS`, music on `A2` at `-16 dB` base gain with `duck_target_db: -18.0`, `fade_in_sec: 0.15`, `fade_out_sec: 0.8`. |

---

## 2. Pre-Edit Shotlist & Cut Plan Template

Write this compact plan before making timeline mutations (and pause for user confirmation during interactive sessions):

```markdown
### Shotlist Plan — <Project Name> (<Aspect>, <Target Duration>s)
- **Brand & Look**: <brand_id> | Log recovery: <yes/no> | LUT: <path @ intensity> | Grade: contrast <x>, sat <y>, temp <z>
- **Audio Target**: Voice `-14.0 LUFS` (`A1`/`V1`) | Music `<media_id>` on `A2` at `-16 dB`, ducked to `-18 dB`
- **Captions (`T1`)**: preset `<preset>`, `<2-4>` words/burst, `y: 0.20`, highlight `<hex>`

| Time | Track(s) | Shot / Action | Motion / Framing | Overlay / Audio Cue |
| --- | --- | --- | --- | --- |
| `0.0–2.4s` | `V1`, `G1` | Hook line: "<first words>" | `scale: 1.08` punch-in | `chapter-title-solar` (`y: -0.24`) |
| `2.4–5.8s` | `V1` | Core claim (silences >0.25s trimmed) | `scale: 1.00` reset | `T1` captions (`y: 0.20`) |
| `5.8–8.2s` | `V2` | B-roll cutaway `<media_id>` | `ken_burns: true` / `gain_db: -99` | `stat-card-solar` (`y: -0.22`) |
| `8.2–12.0s` | `V1`, `G1` | Proof + CTA | `scale: 1.10` emphasis | `comment-bubble-solar` (`y: -0.20`), music fade out `0.8s` |
```

---

## 3. Hook & Visual Momentum Patterns

Choose a deliberate structural device instead of dumping clips end-to-end:

1. **Result-First Hook**: Place the single best visual payoff or boldest spoken sentence at `0.0s` (`source_in_sec` trimmed straight to onset), then jump into the explanation.
2. **Alternating Energy Spine**: On talking-head cuts, alternate `scale: 1.00` ↔ `1.08–1.12` across consecutive jump cuts so cuts read as intentional camera energy rather than accidental stutters.
3. **Visual Proof on Every Claim**: Whenever the speaker states a number, tool, or step, land a `V2` screen/B-roll cutaway or a `G1` `stat-card-solar` / `progress-bar-solar` within `0.10s` of the spoken beat.
4. **J/L Cut Bridges**: Keep `V1`/`A1` voice continuous underneath `V2` B-roll cutaways (`1.5–3.0s`) and let audio lead visual cuts by `0.15–0.30s` on topic shifts.
5. **Beat-Locked Rhythm**: When `manifest.beats` is populated on `A2` music, snap `V1`/`V2` cuts and `G1` entrances within `±0.10s` of a beat.

---

## 4. BANNED Amateur Defaults

Reject and fix any of these before delivering:

- **Empty frame 0 / slow fade-in**: Starting with black frames, dead air, or a `>0.5s` lead gap before the hook lands.
- **Unedited dead air**: Leaving pauses `>0.35s` in a short-form talking-head spine without a deliberate dramatic reason.
- **Static visual stretch `>3.5s`**: Holding an unchanged still image (`ken_burns: false`) or static shot with zero B-roll, zoom shift, or graphic reveal.
- **Caption/GFX collisions**: Placing `G1` graphics at default `y: 0.0` or `y: 0.20` where they crash directly into `T1` captions or the speaker's face.
- **Social chrome burial**: Allowing captions or lower-thirds to drop into the bottom `22%` (`y > 0.35`) or right `15%` (`x > 0.20`) on `9:16` exports.
- **100% LUT crush / raw Log gray**: Shipping unrecovered flat Log footage (`is_log_footage: true` with neutral grade) OR slamming a creative LUT to `lut_intensity: 1.0` that clips skin highlights.
- **Music drowning voice**: Leaving `A2` music at `0 dB` or without `ducking_enabled: true` when speech exists on `V1`/`A1`.
- **Invented numbers or generic filler**: Putting made-up stats (`"300% FASTER"`) or placeholder text (`"LOREM IPSUM"`, `"Speaker Name"`) on screen when the script/brand never stated them.
- **Re-authoring existing starter GFX**: Writing verbose custom inline HTML when `chapter-title-solar`, `stat-card-solar`, `comment-bubble-solar`, `lower-third-solar`, or `progress-bar-solar` already fits cleanly.
