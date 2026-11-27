---
name: audio-and-mix
description: Balances broadcast -14 LUFS voice loudness, A2 music beds with sidechain ducking, click-free clip fades, B-roll scratch muting, J/L audio bridges, and frame-locked sound event discipline in Solaris. Use when normalizing dialog/voiceover, adding background music, enabling speech-driven ducking, fixing loudness QA warnings, or smoothing jump-cut audio transitions. Not for video color grading or caption styling.
---

# Solaris Audio & Mix Craft

Solaris routes `V1` (when unmuted and no `A1` voice track overrides it), `A1` (`VOICE`), and `A2` (`MUSIC`) through a playhead-synced Web Audio engine in preview and FFmpeg `loudnorm`/`sidechaincompress` in export.

## 1. Inspect Loudness & Beats Before Mixing

1. Call `media.get_manifest` for every audio-bearing asset (`V1`, `A1`, `A2`, `V2`).
2. Read `integrated_lufs`, `silences`, and `beats`.
3. Confirm speech lives on **`V1`** (talking-head) or **`A1`** (voiceover narration), and music lives strictly on **`A2`** (placing voice on `A2` causes it to be ducked).
4. Use `beats` on `A2` to align `V1`/`V2` cuts and `G1` entrances within `±0.10s` of musical downbeats, and match music energy to the 4-stage arc (Intro → Build → Drop → Outro).

## 2. Voice First (`-14.0 LUFS` Broadcast Target)

- Call **`audio.normalize_lufs(project_id, target_lufs=-14.0)`** to automatically compute and apply gain deltas across unmuted `V1`/`A1` speech clips.
- **Safety Clamp Awareness**: Solaris clamps per-clip `gain_db` to **`[-12.0, +12.0]` dB** and skips muted tracks or clips `<= -90 dB`.
  - Effective loudness target checked by QA: `integrated_lufs + gain_db` within **`±2.5 LUFS`** of `-14.0` (export targets `-14.0 LUFS` integrated / `-1.5 dBTP` true peak).
  - Never copy a raw gain number blindly from another project—always compute from the asset's measured `integrated_lufs`. If a raw recording is quieter than `-26.5 LUFS`, `audio.normalize_lufs` stops safely at `+12.0 dB`—do not loop it repeatedly; note the quiet source recording in your final report.

## 3. Music Bed & Sidechain Ducking (`A2`)

1. Place music on **`a2`** via `timeline.place_clip`:
   - Clamp `duration_sec` to the exact end of the visual/voice timeline (never leave an orphan music tail after black).
   - Set base **`gain_db`** between **`-14.0` and `-18.0` dB** (QA requires `gain_db <= -8.0` whenever speech is present).
   - **Phone-speaker clarity**: Favor tracks with clean mid-range presence—avoid beds with `>55%` of their energy below `150 Hz`, which vanish on phone speakers while muddying dialog.
2. Call **`audio.enable_ducking(project_id, duck_target_db=-18.0)`** (valid sweet spot: `-16.0` to `-20.0 dB`; QA requires `<= -12.0 dB`) so FFmpeg's `sidechaincompress` automatically dips `A2` whenever `V1`/`A1` speech is active.
   - **Unducked SFX Routing on `A2` & Ordering Hazard**: `A2` clips with `ducking_enabled: false` bypass sidechain compression cleanly (`unducked_music_labels`), making them ideal for transient SFX. **CRITICAL ORDERING RULE**: `audio.enable_ducking` enables ducking on **ALL** `A2` clips, so always call `audio.enable_ducking` **BEFORE** placing unducked SFX on `A2` (or reset `ducking_enabled: false` on SFX clips via `audio.set_mix` afterward).
3. Shape bookends with **`audio.set_mix`**:
   - Music entrance: `fade_in_sec: 0.15` to `0.30`.
   - Music exit: `fade_out_sec: 0.60` to `1.20` (clamped `<= duration_sec`, never negative).

## 4. Sound Event Discipline, J/L Cuts & Scratch-Audio Hygiene

- **Imported SFX Only (No Built-in Synth)**: Solaris has **no built-in SFX synthesizer**—all SFX must be imported audio files in `media_pool`, and `<audio>` tags are forbidden inside `G1` HTML. Place SFX on `A2` with `ducking_enabled: false` *after* running `audio.enable_ducking`.
- **A Sound Equals a Visible Event**: When placing SFX assets (click, tick, pop, whoosh, hit, shimmer, riser), tie every sound to a specific visible change on screen (cursor click, stat stamp, camera whip, logo drop). Never fire an SFX on a static beat with nothing changing visually.
- **Peak on the Frame (`+16.7ms`)**: Light arrives before sound—align an SFX peak to its visual cut or `+16.7ms` (half a 30fps frame) *after* the visual arrival (within `±50ms`, slightly after rather than early).
- **Pre-Hit Breath & One Big Hit**: Use one consistent UI sound per role across the edit (rotating random variants reads as unstable). Reserve a single big impact hit for the drop or end-card mark (never stack a whoosh on top of a drop impact), preceded by a **`0.25–0.30s` pre-hit breath** (subtle music dip before the impact lands).
- **Mute B-Roll Scratch Audio**: Set `gain_db: -99` on `V2` B-roll clips unless natural ambient SFX is intentional.
- **Eliminate Jump-Cut Clicks**: Apply subtle **`fade_in_sec: 0.05`** and **`fade_out_sec: 0.05–0.12`** via `audio.set_mix` on hard-trimmed speech segments.
- **J/L Cuts**: Let `V1`/`A1` dialog run continuously underneath `V2` cutaways so the audio spine glues visual changes together.

## 5. Verification & Honest Disclosure

1. Run `project.run_qa(project_id, auto_fix=true)` to confirm zero speech loudness or ducking warnings remain.
2. **Epistemic honesty**: Always tell the user plainly that loudness (`-14 LUFS`), ducking (`-18 dB`), and fades were **verified from numeric manifest/mix telemetry, not listened to by ear**.
