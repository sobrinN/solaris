---
name: talking-head-shorts
description: Pacing, jump-cut, punch-in zoom, smart 9:16 framing, and B-roll cutaway rules for vertical talking-head shorts, Reels, and TikToks.
---

# Talking-Head Shorts Craft

1. **0–3s Retention Hook**:
   - Start immediately on the speaker's first spoken word or strongest visual beat (`source_in_sec` aligned to first speech onset).
   - Pair the hook with a `106%–112%` punch-in (`timeline.set_transform` with `scale: 1.08`) or a bold chapter title card.

2. **Breath-Safe Jump Cuts (`razor`)**:
   - Read `silences` from `media.get_manifest`.
   - Cut silences longer than `0.25s`, leaving a **`0.12s` breath-safe pad** before and after words so cuts feel crisp rather than clipped.
   - Alternate scale (`1.00` ↔ `1.08`–`1.14`) across consecutive jump cuts to disguise edits cleanly.

3. **Smart `9:16` Reframing**:
   - When `canvas.aspect_preset` is `vertical9x16` and source video is `16:9`, keep the speaker centered (`transform.x: 0.0`, `scale >= 1.0`) and verify eyes sit in the upper third via `preview.capture_frame`.

4. **B-Roll Cadence**:
   - Place B-roll inserts on `V2` every `3.0–5.0s` for `1.5–3.0s` each while keeping A-roll audio continuous underneath (J/L cut feel).
   - Mute or attenuate (`gain_db: -99`) B-roll clip audio unless intentional SFX/ambience is needed.
