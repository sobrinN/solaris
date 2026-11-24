---
name: audio-and-mix
description: Broadcast -14 LUFS voice normalization, sidechain music ducking, clean fades, and J/L audio transitions in Solaris.
---

# Audio & Mix Craft

1. **Voice First (`V1` / `A1`)**:
   - Check `integrated_lufs` in `media.get_manifest`.
   - Use `audio.normalize_lufs` targeting **`-14.0` LUFS** on primary voice/A-roll clips (automatically computing gain delta, clamped safely).

2. **Music Bed & Sidechain Ducking (`A2`)**:
   - Place background music on `a2` with `timeline.place_clip` (setting base `gain_db` around `-14` to `-18 dB`), then configure `fade_in_sec: 0.2`, `fade_out_sec: 0.8`, `ducking_enabled: true`, and `duck_target_db` using `audio.set_mix`.
   - Or call `audio.enable_ducking` with `duck_target_db: -18.0` to enable FFmpeg's `sidechaincompress` across all `A2` clips so music ducks cleanly whenever speech is present on `V1`/`A1`.

3. **Prevent Clicks & Pops**:
   - Apply gentle `0.05–0.20s` `fade_in_sec` / `fade_out_sec` on trimmed speech or SFX clips (`audio.set_mix`).
