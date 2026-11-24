---
name: director-core
description: Core directing workflow for Solaris. Inspects MediaIntelligenceManifest and BrandPlaybook, builds a shotlist/hook plan, executes spine edits (V1/A1) before parallel specialist fan-out (V2/FX/T1/G1/A2), and verifies output with multimodal QA.
---

# Director Core Workflow

1. **Inspect State First**:
   - Call `project.get` and `brand.get_playbook` to understand canvas aspect (`9:16`, `16:9`, `1:1`), `brand_md`, `tokens`, `pacing_rules`, and `default_lut`.
   - Call `media.get_manifest` for every asset in `media_pool`. Check `duration_sec`, `is_log_footage`, `silences`, `beats`, `integrated_lufs`, and `transcript`.
   - If a primary speech asset has `has_audio: true` and an empty `transcript`, call `media.transcribe` before planning captions or cuts.

2. **Build a Tight Hook & Shotlist Plan**:
   - **0.0–3.0s Hook**: Open with the strongest visual or statement. No slow fades or dead air at `0.0s`.
   - **Spine First (`V1` + `A1`)**: Lock `v1` and `a1` (`project.lock_track`), place A-roll/voice clips, trim dead silences (`> 0.25s`) while preserving `0.12s` breath pads, and release locks before specialist fan-out.

3. **Parallel Specialist Fan-Out**:
   - `broll` (`v2`): Insert B-roll cutaways or stills (`ken_burns: true`) every `3.0–5.0s` to maintain visual momentum.
   - `grade` (`fx`): Recover Log footage to Rec.709, apply brand `.cube` LUT (`lut_intensity` `0.65–0.85`), and balance contrast/skin tones.
   - `caps` (`t1`): Generate 2–4 word bursts inside the 80% safe zone using `captions.generate_from_transcript`.
   - `gfx` (`g1`): Search `assets.search` first; place brand-tokenized lower-thirds, title cards, or stat callouts without overlapping captions.
   - `mix` (`a2`): Normalize speech to `-14 LUFS`, place music bed on `a2`, and enable `-18 dB` voice-driven sidechain ducking.

4. **Multimodal QA Pass**:
   - Run `project.run_qa` (with `auto_fix: true`) alongside `color.get_scope_stats` and `preview.capture_frame` at key timestamps (hook `0.5s`, mid-cut, climax) to verify safe-zone clearance, -14 LUFS loudness, contrast, and typography legibility.
