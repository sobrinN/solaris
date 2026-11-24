---
name: color-and-look
description: Applies Log-to-Rec.709 color recovery, Lift/Gamma/Gain balancing, exposure/white-balance correction, skin-tone protection, and .cube 3D LUT blending in Solaris. Use when grading S-Log3/Log-C/V-Log footage, matching B-roll to A-roll, applying a Brand Playbook LUT, or fixing washed-out, dark, or over-saturated frames. Not for audio mixing or GFX styling.
---

# Solaris Color & Look Craft

Solaris renders color grades identically in the real-time WebGL2 Program Monitor (with half-texel 3D `.cube` LUT sampling) and FFmpeg export.

## 1. Inspect Real State (Honest Tool Behavior)

1. Call `color.get_scope_stats` and `brand.get_playbook`.
   - **Important**: `color.get_scope_stats` returns metadata only (`active_grade`, `brand_default_lut`, `log_footage_assets`). It does **not** return numeric waveform/vectorscope pixel histograms.
2. Call `media.get_manifest` to verify `is_log_footage` and `color_space` per clip.
3. Call `preview.capture_frame` on at least 2 representative frames (e.g., primary A-roll face + key B-roll cutaway) to **visually inspect** exposure, shadow floor, highlight roll-off, and skin tones before and after grading.

## 2. Global vs `FX` Clip Grading

- **Standard path (global grade)**: Call `color.set_grade` **omitting both `track_id` and `clip_id`** to update `project.active_grade`.
- **Clip-scoped `FX` grade**: Pass **both** `track_id: "fx"` and `clip_id` *only* if `project.get` already contains an existing `ClipKind::Grade` clip on `fx`. (Passing only one of the two errors; Solaris does not expose a separate tool to create new `fx` clips via MCP.)

## 3. Numerical Starting Points (`color.set_grade`)

Always start from disciplined, repeatable numbers—then refine visually via `preview.capture_frame`:

| Scenario | `contrast` | `saturation` | `exposure` | `lift` (`{r,g,b}`) | `gamma` (`{r,g,b}`) | `gain` (`{r,g,b}`) | `lut_intensity` |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **Log footage, no LUT** | `1.16–1.24` | `1.14–1.22` | `0.00` to `+0.08` | `{-0.02, -0.02, -0.015}` | `{0.0, 0.0, 0.0}` | `{0.03, 0.03, 0.02}` | `0.0` |
| **Log footage + Brand `.cube` LUT** | `1.04–1.10` | `1.02–1.08` | `0.00` | `{-0.01, -0.01, -0.01}` | `{0.0, 0.0, 0.0}` | `{0.01, 0.01, 0.01}` | **`0.65–0.85`** |
| **Rec.709 footage + Brand LUT** | `1.02–1.06` | `1.00–1.05` | `0.00` | `{0.0, 0.0, 0.0}` | `{0.0, 0.0, 0.0}` | `{0.0, 0.0, 0.0}` | **`0.45–0.70`** |
| **Rec.709 clean editorial polish** | `1.05–1.10` | `1.04–1.10` | `0.00` | `{-0.01, -0.01, -0.008}` | `{0.0, 0.0, 0.0}` | `{0.02, 0.02, 0.01}` | `0.0` |

### Parameter Guardrails
- **`temp` & `tint`**: Keep restrained (`temp` in `[-0.12, +0.15]`, `tint` in `[-0.08, +0.08]`) to correct greenish LEDs or cool daylight without turning skin orange or magenta.
- **`exposure`**: Adjust in small `±0.05`–`±0.15` steps (`-1.0` to `+1.0` valid range) rather than crushing shadows with `lift` or blowing highlights with `gain`.
- **Brand LUT blending**: Whenever `brand.get_playbook` includes `default_lut`, pass `lut_path` and blend at `0.65–0.85` (never blindly `1.0`, which frequently clips specular highlights and oversaturates skin).

## 4. Visual Verification Loop

1. After `color.set_grade`, call `preview.capture_frame` on the hero face shot and a bright/outdoor cutaway.
2. Check for:
   - **Crushed blacks**: shadow detail turning into muddy solid blocks → raise `lift` toward `0.0` or lower `contrast`.
   - **Clipped highlights / waxy skin**: forehead or sky losing texture → lower `gain`, `exposure`, or `lut_intensity` by `0.10–0.15`.
   - **Unnatural skin cast**: orange/magenta/green shift on faces → nudge `temp` / `tint` back toward `0.0` and dial `saturation` down `0.05`.
3. If an aggressive grade degrades the image, revert immediately with `project.undo` and apply a smaller adjustment.

## 5. BANNED Color Defaults

- Shipping flat, milky gray `is_log_footage: true` clips with default neutral `1.0` contrast/saturation.
- Slamming `lut_intensity: 1.0` on top of high contrast (`>1.20`) and high saturation (`>1.25`).
- Grading blind without calling `preview.capture_frame` to check real skin tones and highlight roll-off.
- Claiming you "read the luma waveform" from `color.get_scope_stats` (only report what you actually inspected in `preview.capture_frame`).
