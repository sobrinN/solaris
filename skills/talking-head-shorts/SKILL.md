---
name: talking-head-shorts
description: Cuts and paces vertical 9:16 talking-head shorts, Reels, and TikToks in Solaris using breath-safe jump cuts, 0–3s retention hooks, alternating punch-in zooms, smart 9:16 reframing, B-roll J/L cutaways, and spoken-cue transparent GFX overlays. Use when editing speaker-led footage, podcast clips, creator reels, or removing silences on V1/V2. Not for faceless slideshows or non-Solaris editors.
---

# Solaris Talking-Head Shorts Craft

Use this skill whenever the primary spine is a human speaker on `V1` (typically `vertical9x16`, `1080×1920`, 30fps).

## 1. Pre-Cut Inspection

1. Call `media.get_manifest` for the A-roll asset. Read `silences`, `transcript`, `duration_sec`, and `integrated_lufs`.
2. If `has_audio: true` and `transcript` is empty, call `media.transcribe` before planning cuts.
3. Call `media.extract_keyframes` at `[0.5, mid]` to verify speaker eye-line, headroom, and framing before deciding `scale` and `x/y` offsets.

## 2. 0–3s Retention Hook

- **Frame 0 starts on speech or motion**: Set the opening `V1` clip's `source_in_sec` directly to the first spoken word onset (minus `0.08–0.12s` breath lead) so there is zero dead air and zero lead gap (`start_sec: 0.0`).
- **Visual emphasis**: Pair the opening `0.0–2.5s` hook with either:
  - A **`1.08–1.12` punch-in** via `timeline.set_transform` on `v1`, OR
  - A bold `chapter-title-solar` (or custom kinetic slam / myth-buster overlay from `skill://captions-and-gfx/references/move-playbook.md`) placed on `g1` (`gfx.place`) and positioned clear of the speaker's face via `timeline.set_transform(track_id="g1", y=-0.25, scale=0.92)`.

## 3. Breath-Safe Jump-Cut Math (`V1`)

Remove dead pauses while keeping delivery natural—never chop consonants or leave `> 0.40s` gaps on `V1`:

1. Read `silences` (`[{ start_sec, end_sec }]`, computing `end_sec - start_sec` for duration) from `media.get_manifest`.
2. **Cut threshold**: Remove any silence with `duration_sec > 0.25`.
3. **Breath-safe pad**: Keep **`0.12s`** before speech resumes and **`0.12s`** after a phrase ends (minimum `0.24s` kept around a trimmed silence) so cuts breathe cleanly without robotic clicks.
4. **Contiguous timeline placement**:
   - **Building fresh**: Place each kept speech segment sequentially on `v1` with `timeline.place_clip`, setting each clip's `start_sec` to the exact end (`prev_start + prev_duration`) of the previous clip so there are **0.0s V1 gaps**.
   - **Refining an existing clip**: Use `timeline.split_clip` at the silence boundaries, `timeline.delete_clip` on the dead middle segment, and `timeline.trim_clip` (`new_start_sec`) to ripple subsequent clips closed.
5. **Disguise jump cuts with alternating punch-ins**:
   - Alternate `scale` across consecutive `V1` clips: `1.00` → `1.08` → `1.00` → `1.12` via `timeline.set_transform`.
   - Keep `scale` between `1.00` and `1.15` (higher values soften 1080p footage noticeably).
   - Keep slight `0.05s` `fade_in_sec` / `fade_out_sec` via `audio.set_mix` on hard-trimmed speech clips to prevent audio clicks.

## 4. Smart `9:16` Reframing

- When `canvas.aspect_preset` is `vertical9x16` and raw A-roll is `16:9` landscape:
  - Keep the speaker centered horizontally (`x: 0.0`) unless `media.extract_keyframes` shows an off-center composition—adjust `x` in `[-0.18, +0.18]` to keep eyes centered.
  - Keep eyes in the **upper third** of the frame so `T1` captions (`y: 0.20`) sit cleanly over the chest/torso, never across the mouth or chin.
  - Verify framing with `preview.capture_frame` at `0.5s` and after the first punch-in cut.

## 5. B-Roll Cutaway Cadence (`V2`) & Spoken-Cue Overlays (`G1`)

- **B-Roll Cutaways (`V2`)**: Insert cutaways (screen recordings, product footage, or `ken_burns: true` stills) every **`3.0–5.0s`** for **`1.5–3.0s`** each so no talking-head stretch stalls visually. Keep `V1` audio continuous underneath (authentic J/L cut momentum) and pass `gain_db: -99` on `V2` cutaways unless intentional ambience is needed.
- **Spoken-Cue Transparent Overlays (`G1`)**: Scan the transcript and match high-value spoken beats to visual moves from `skill://captions-and-gfx/references/move-playbook.md`:
  - Spoken number/metric / growth → `stat-card-solar`, `kpi-tiles`, `bar-race`, or `follower-ticker-chip`
  - Myth busted ("everyone says X, actually Y") → `strike-correct` / `verdict-card`
  - List point / chapter / pull-quote → `listicle-number`, `chapter-tag`, `headline-highlight`, or `progress-bar-solar`
  - Viewer quote / alerts / CTA ("comment GUIDE...") → `notify-stack`, `comment-bubble-solar`, `comment-keyword`, or `follow-card`
  - **Face-Safe Overlay Craft**: Keep `9:16` overlays in the **top clearance (`y: -0.22` to `-0.28`) or under-chin band**—never inside the center `40%` face band. Over live `V1`/`V2` footage, use **opaque `88–92%` card fills** (`rgba(..., 0.88–0.92)`) for instant legibility, and apply **zero full-screen vignette or grain** on transparent `G1` overlays (so the speaker's skin and exposure stay clean).
  - **Cadence & Placement Law**: Place at most **one move per idea worth seeing**, **never two floating overlays within `4.0s`**, enter cleanly from empty at `t=0` and exit in the last `0.30–0.40s`, and always offset via `timeline.set_transform(track_id="g1", y=-0.22 to -0.28)` so overlays sit clear of the face band and `T1` captions (`y: 0.20`).

## 6. BANNED Talking-Head Defaults

- Leaving `> 0.35s` silences or throat-clearing at `0.0s`.
- Leaving uncovered gaps `> 0.40s` between `V1` segments (causes black flashes and fails QA).
- Using the exact same `scale: 1.0` on 4+ back-to-back jump cuts without a punch-in or B-roll cover.
- Placing `G1` cards at `y: 0.0` (blocks speaker's face) or `y: 0.20` (collides with `T1` captions), or stacking two overlays within `4.0s`.
- Leaving unmuted `V2` camera scratch audio clashing with the speaker.
