---
name: audio-and-mix
description: Balances broadcast -14 LUFS voice loudness, A2 music beds with sidechain ducking, click-free clip fades, B-roll scratch muting, and J/L audio bridges in Solaris. Use when normalizing dialog/voiceover, adding background music, enabling speech-driven ducking, fixing loudness QA warnings, or smoothing jump-cut audio transitions. Not for video color grading or caption styling.
---

# Solaris Audio & Mix Craft

Solaris routes `V1` (when unmuted and no `A1` voice track overrides it), `A1` (`VOICE`), and `A2` (`MUSIC`) through a playhead-synced Web Audio engine in preview and FFmpeg `loudnorm`/`sidechaincompress` in export.

## 1. Inspect Loudness Before Mixing

1. Call `media.get_manifest` for every audio-bearing asset (`V1`, `A1`, `A2`, `V2`).
2. Read `integrated_lufs`, `silences`, and `beats`.
3. Confirm speech lives on **`V1`** (talking-head) or **`A1`** (voiceover narration), and music lives strictly on **`A2`** (placing voice on `A2` causes it to be ducked).

## 2. Voice First (`-14.0 LUFS` Broadcast Target)

- Call **`audio.normalize_lufs(project_id, target_lufs=-14.0)`** to automatically compute and apply gain deltas across unmuted `V1`/`A1` speech clips.
- **Safety Clamp Awareness**: Solaris clamps per-clip `gain_db` to **`[-12.0, +12.0]` dB** and skips muted tracks or clips `<= -90 dB`.
  - Effective loudness target checked by QA: `integrated_lufs + gain_db` within **`±2.5 LUFS`** of `-14.0`.
  - If a raw recording is quieter than `-26.5 LUFS`, `audio.normalize_lufs` stops safely at `+12.0 dB`—do not loop it repeatedly; note the quiet source recording in your final report.

## 3. Music Bed & Sidechain Ducking (`A2`)

1. Place music on **`a2`** via `timeline.place_clip`:
   - Clamp `duration_sec` to the exact end of the visual/voice timeline (never leave an orphan music tail after black).
   - Set base **`gain_db`** between **`-14.0` and `-18.0` dB** (QA requires `gain_db <= -8.0` whenever speech is present).
2. Call **`audio.enable_ducking(project_id, duck_target_db=-18.0)`** (valid sweet spot: `-16.0` to `-20.0 dB`; QA requires `<= -12.0 dB`) so FFmpeg's `sidechaincompress` automatically dips `A2` whenever `V1`/`A1` speech is active.
3. Shape bookends with **`audio.set_mix`**:
   - Music entrance: `fade_in_sec: 0.15` to `0.30`.
   - Music exit: `fade_out_sec: 0.60` to `1.20` (clamped `<= duration_sec`, never negative).

## 4. Clean Cut Transitions & Scratch-Audio Hygiene

- **Mute B-Roll Scratch Audio**: Set `gain_db: -99` on `V2` B-roll clips unless natural ambient SFX is intentional.
- **Eliminate Jump-Cut Clicks**: Apply subtle **`fade_in_sec: 0.05`** and **`fade_out_sec: 0.05–0.12`** via `audio.set_mix` on hard-trimmed speech segments.
- **J/L Cuts**: Let `V1`/`A1` dialog run continuously underneath `V2` cutaways so the audio spine glues visual changes together.

## 5. Verification & Honest Disclosure

1. Run `project.run_qa(project_id, auto_fix=true)` to confirm zero speech loudness or ducking warnings remain.
2. **Epistemic honesty**: Always tell the user plainly that loudness (`-14 LUFS`), ducking (`-18 dB`), and fades were **verified from numeric manifest/mix telemetry, not listened to by ear**.
