# Solaris Tool, Coordinate & Resource Reference

## Table of Contents

1. [Complete 34-Tool Surface](#1-complete-34-tool-surface)
2. [Track Architecture & Role Hints](#2-track-architecture--role-hints)
3. [Transform2D Normalized Coordinate System](#3-transform2d-normalized-coordinate-system)
4. [MCP Resources & Export Status Polling](#4-mcp-resources--export-status-polling)
5. [Built-In Project Templates](#5-built-in-project-templates)

---

## 1. Complete 34-Tool Surface

External MCP uses dotted names (`project.get`); internal OpenRouter swarm uses underscored names (`project_get`). Both normalize identically in the Rust backend.

### Project & Locking (8 tools)

| Tool | Required Params | Optional Params & Notes |
| --- | --- | --- |
| `project.list` | — | Returns all workspace project summaries (`id`, `name`, `revision`). |
| `project.create` | `name` | `aspect_preset` (`vertical9x16` \| `landscape16x9` \| `square1x1`), `brand_playbook_id`, `prompt`, `template_id` (`talking-head-short` \| `faceless-explainer`). If `template_id` is set and `aspect_preset` omitted, preserves template's native aspect. |
| `project.get` | `project_id` | Full Timeline IR JSON (`tracks`, `clips`, `media_pool`, `active_grade`, `canvas`, `revision`). |
| `project.run_qa` | `project_id` | `auto_fix` (`boolean`). Runs hook, gap, safe-zone, collision, LUFS, ducking, and GFX lint checks. |
| `project.undo` | `project_id` | Reverts latest timeline op while preserving `media_pool[*].intelligence` (transcripts, proxies, waveforms). |
| `project.redo` | `project_id` | Reapplies undone op. |
| `project.lock_track` | `project_id`, `track_id` | Acquires agent lock on `t1`, `g1`, `v2`, `v1`, `fx`, `a1`, or `a2`. Fails if user-locked. |
| `project.unlock_track` | `project_id`, `track_id` | Releases agent lock. Cannot unlock a `User`-locked track. |

### Media & Intelligence (4 tools)

| Tool | Required Params | Optional Params & Notes |
| --- | --- | --- |
| `media.import` | `project_id`, `paths` (`[{path, role_hint?}]`) | Imports local paths into `media_pool` and triggers probing/analysis. Respects `allow_external_media_reads`. |
| `media.get_manifest` | `project_id` | `media_id` (omit for all assets). Returns `duration_sec`, `has_audio`, `is_log_footage`, `silences`, `beats`, `scene_cuts`, `integrated_lufs`, `transcript`, `waveform_peaks_count`. |
| `media.transcribe` | `project_id`, `media_id` | `language`. Runs local FFmpeg whisper or OpenRouter STT and persists word-level timestamps into `manifest.transcript`. |
| `media.extract_keyframes` | `project_id`, `media_id` | `timestamps_sec` (`number[]`). Returns base64 PNG images of raw source media for pre-edit visual scouting. |

### Timeline Editing (5 tools)

| Tool | Required Params | Optional Params & Notes |
| --- | --- | --- |
| `timeline.place_clip` | `project_id`, `track_id`, `clip_id`, `media_id`, `start_sec`, `duration_sec` | `label`, `source_in_sec`, `ken_burns` (`bool` for stills), `scale` (`> 0`), `gain_db`. Valid tracks: `v1`, `v2`, `a1`, `a2`. |
| `timeline.split_clip` | `project_id`, `track_id`, `clip_id`, `split_at_timeline_sec`, `right_clip_id` | Splits an existing clip into two contiguous clips; preserves `source_in_sec` offset on right half. |
| `timeline.trim_clip` | `project_id`, `track_id`, `clip_id` | `new_start_sec`, `new_duration_sec`, `new_source_in_sec`. Slips/trims clip boundaries without deleting it. |
| `timeline.delete_clip` | `project_id`, `track_id`, `clip_id` | Removes clip from track. |
| `timeline.set_transform` | `project_id`, `track_id`, `clip_id` | `x`, `y`, `scale`, `rotation_deg`, `opacity`. Works on visual, caption (`t1`), and GFX (`g1`) clips. |

### Color & Look (2 tools)

| Tool | Required Params | Optional Params & Notes |
| --- | --- | --- |
| `color.set_grade` | `project_id` | `track_id` & `clip_id` (both or neither; omit to set global `project.active_grade`), `lift`/`gamma`/`gain` (`{r,g,b}`), `contrast`, `saturation`, `exposure`, `temp`, `tint`, `lut_path` (`string \| null`), `lut_intensity` (`0.0–1.0`). |
| `color.get_scope_stats` | `project_id` | Returns `{ active_grade, brand_default_lut, log_footage_assets }`. Does **not** return pixel luma/vectorscope histograms—use `preview.capture_frame` to visually inspect exposure and skin tones. |

### Captions, Assets & GFX (7 tools)

| Tool | Required Params | Optional Params & Notes |
| --- | --- | --- |
| `captions.generate_from_transcript` | `project_id` | `media_id`, `preset` (`neon_glow` \| `mono_caps` \| `karaoke` \| `minimal`), `words_per_burst` (`1–6`). Maps against `V1` and `A1`, splits across `>0.6s` silences, sets `y: 0.20`, and replaces prior auto-captions. |
| `captions.place_script` | `project_id`, `segments` (`[{text, start_sec, duration_sec}]`) | `preset`, `words_per_burst` (`1–6`), `y` (default `0.24`), `replace_existing` (default `true`). Places word-synced kinetic captions on `T1` without requiring an audio transcript (ideal for zero-asset programmatic videos). |
| `captions.set_style` | `project_id`, `preset` | `font_family`, `highlight_color` (hex). Restyles all `T1` clips. |
| `assets.search` | — | `query`, `category`. Matches asset `id`, `name`, and `tags`. |
| `assets.save` | `id`, `name`, `html_source` | `category`, `tags`, `props_schema`, `default_props`, `default_duration_sec`. Lints HTML and validates ID regex `[a-zA-Z0-9_-]{1,64}`. |
| `gfx.place` | `project_id`, `clip_id`, `label`, `start_sec`, `duration_sec` | `asset_id` + `props` (deep-merged over `default_props`), OR `inline_html`. Enforces `lint_gfx_html` server-side automatically. Always places at default `x:0, y:0, scale:1`—call `timeline.set_transform` afterward for floating cards (keep full-canvas programmatic scenes at `(0,0)`). |
| `gfx.lint_html` | `html` | Validates seekable & sandbox rules (`class="clip"`, `data-start`, `data-duration`, no banned APIs/URLs, `<= 128 KB`). Use to pre-verify or debug rejections (`gfx.place` also runs it automatically). |

### Audio Mix (3 tools)

| Tool | Required Params | Optional Params & Notes |
| --- | --- | --- |
| `audio.set_mix` | `project_id`, `track_id`, `clip_id` | `gain_db`, `ducking_enabled`, `duck_target_db`, `fade_in_sec` (`>= 0`), `fade_out_sec` (`>= 0`). |
| `audio.normalize_lufs` | `project_id` | `target_lufs` (default `-14.0`). Adjusts unmuted speech/A-roll clip `gain_db` clamped to `[-12, +12]` dB. |
| `audio.enable_ducking` | `project_id` | `duck_target_db` (default `-18.0`). Enables FFmpeg `sidechaincompress` across all `A2` music clips. |

### Brand, Preview & Render (5 tools)

| Tool | Required Params | Optional Params & Notes |
| --- | --- | --- |
| `brand.list_playbooks` | — | Lists all available Brand Playbooks. |
| `brand.get_playbook` | `brand_id` | Returns `brand_md`, `tokens`, `pacing_rules`, `default_lut`, and `reference_media` with notes. |
| `preview.seek` | `project_id`, `time_sec` | Moves the live Studio UI Program Monitor playhead. |
| `preview.capture_frame` | `project_id`, `time_sec` | Returns a composited PNG (V1/V2 + transforms + ColorGrade/LUT + Chromium G1/T1 overlays, OR programmatic overlay-only G1/T1 frames over the brand backdrop when no V1/V2 media is present; headless Chromium is required to render `G1`/`T1` overlays). |
| `render.export` | `project_id` | `format` (`mp4` \| `mov_pro_res4444` \| `webm_alpha`), `quality` (`draft` \| `standard` \| `high`), `target_lufs` (clamped `-70` to `-5`, default `-14.0`), `output_name`. Starts an async job and returns `job_id`. |

---

## 2. Track Architecture & Role Hints

| Track ID | Code | Allowed Content | Z-Order / Audio Behavior |
| --- | --- | --- | --- |
| `t1` | `TEXT` | Word-synced captions (`ClipKind::Caption`) | Topmost visual layer (always composites *above* `g1`). |
| `g1` | `GFX` | Programmable HTML/CSS/GSAP overlays | Composites above `v2`/`v1` and under `t1`. |
| `v2` | `B-ROLL` | Cutaways, screen recordings, Ken Burns stills | Composites over `v1`. Set `gain_db: -99` unless ambience is desired. |
| `v1` | `A-ROLL` | Primary speaker / visual spine | Base visual layer + primary dialog audio when `a1` is empty. |
| `fx` | `GRADE` | Adjustment-layer `ClipKind::Grade` | Applies grade over its `[start, end]` range; no MCP creation tool exists (edit global grade via `color.set_grade` without `track_id`/`clip_id`). |
| `a1` | `VOICE` | Dedicated voiceover / narration audio | Primary speech bus; feeds sidechain ducking key. |
| `a2` | `MUSIC` | Beat track, score, SFX bed | Ducked automatically under `V1`/`A1` speech when `ducking_enabled: true`. |

### `media.import` `role_hint` Values
Use canonical hints so auto-transcription and routing match intent:
- `a_roll`: primary talking-head video (`V1`).
- `voice` / `voiceover` / `vo` / `dialogue` / `speech` / `narration`: narration audio (`A1`, auto-transcribed).
- `b_roll` / `broll` / `cutaway` / `insert`: cutaway video/images (`V2`).
- `music` / `bgm` / `beat` / `soundtrack`: music bed (`A2`, never transcribed).
- `sfx` / `ambience`: sound effects.
- `reference`: style/script reference material.

---

## 3. Transform2D Normalized Coordinate System

All clip transforms (`timeline.set_transform`) use **canvas-normalized fractions**:
- `x: 0.0`, `y: 0.0` = exact canvas center.
- Positive `x` moves right (`+0.1` = 10% of canvas width); positive `y` moves down (`+0.20` = 20% down from center).
- `scale: 1.0` = native fit; `1.08` = 108% punch-in; `0.85` = 85% card/PiP scale.
- `opacity`: `0.0` to `1.0` (`>= 0.85` required for readable captions).
- `rotation_deg`: degrees (`-180` to `180`).

### Safe-Zone Bounds Enforced by Solaris QA
- **Captions (`t1`)**: `y` in `[-0.35, 0.35]` (default `0.20`, anchored safely above the bottom 22% Reels/TikTok chrome); `x` in `[-0.20, 0.20]` for `9:16` (`[-0.35, 0.35]` for `16:9` / `1:1`); `scale` in `[0.75, 1.35]`.
- **GFX (`g1`)**: `y` in `[-0.35, 0.35]`; `x` in `[-0.20, 0.20]` for `9:16` (`[-0.35, 0.35]` for `16:9` / `1:1`); `scale` in `[0.50, 1.50]`.
- **Caption/GFX Separation**: Whenever a `g1` clip overlaps a `t1` clip in time, keep `|gfx.y - cap.y| >= 0.08` (e.g., captions at `y: 0.20`, stat/title card at `y: -0.22`).

---

## 4. MCP Resources & Export Status Polling

Readable MCP resources (`resources/read`):
- `skill://<name>` and `skill://<name>/references/<file>.md` — all bundled Solaris craft skills and reference docs.
- `solaris://projects` — workspace project list JSON.
- `solaris://project/{project_id}` — live project Timeline IR JSON.
- `solaris://brands` — all Brand Playbooks JSON.
- `solaris://templates` — built-in starter templates metadata JSON.
- `solaris://assets/pack` — portable `solaris.asset-pack/v1` manifest of all saved GFX assets.

**Export Status Note**: `render.export` starts an asynchronous export and returns `{ job_id, state }`. There is no separate `render.get_status` MCP tool; poll `GET <SOLARIS_SERVER_URL>/api/v1/exports/{job_id}` (with bearer auth if configured) or instruct the user to download from Studio's Export modal.

---

## 5. Built-In Project Templates

| `template_id` | Native Aspect | Default Structure |
| --- | --- | --- |
| `talking-head-short` | `vertical9x16` (`1080×1920`, 30fps) | Pre-wired 9:16 short with hook title (`chapter-title-solar`), punch-in pacing cues, and social safe-zone captions. |
| `faceless-explainer` | `landscape16x9` (`1920×1080`, 30fps) | Pre-wired 16:9 explainer with chapter title, `lower-third-solar`, voiceover `A1` + ducked `A2` music lanes, and Ken Burns visual rhythm. |
