---
name: color-and-look
description: Log-to-Rec.709 color recovery, Lift/Gamma/Gain balancing, skin-tone protection, and .cube 3D LUT blending using Solaris scope telemetry.
---

# Color & Look Craft

1. **Inspect Camera Color Space**:
   - Call `color.get_scope_stats` and check `is_log_footage` / `color_space` in `media.get_manifest`.

2. **Log-to-Rec.709 Base Recovery (`color.set_grade`)**:
   - Apply grades using `color.set_grade` (omitting `track_id`/`clip_id` for the global project grade, or targeting an `fx` clip).
   - If `is_log_footage` is `true` and no corrective LUT is applied, recover contrast and saturation first:
     - `contrast`: `1.14–1.24`
     - `saturation`: `1.12–1.22`
     - `lift`: RGB object for subtle shadow anchor (e.g. `{ "r": -0.02, "g": -0.02, "b": -0.015 }`)
     - `gamma`: RGB object for midtone balance (e.g. `{ "r": 0.0, "g": 0.0, "b": 0.0 }`)
     - `gain`: RGB object for clean highlight headroom (e.g. `{ "r": 0.03, "g": 0.03, "b": 0.02 }`, never clipping skin or sky)

3. **Brand LUT Blending**:
   - If `brand.get_playbook` defines `default_lut`, pass `lut_path` to `color.set_grade` and blend with `lut_intensity` between `0.65` and `0.85` rather than `1.0` so highlights and skin tones stay natural.

4. **Warmth & Skin-Tone Discipline**:
   - Use small `temp` (`-0.12` to `+0.15`) and `tint` (`-0.08` to `+0.08`) adjustments; verify skin tones with `preview.capture_frame`.
