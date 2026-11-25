---
name: director-core
description: Orchestrates end-to-end video editing in Solaris across all 7 timeline tracks (T1, G1, V2, V1, FX, A1, A2) via MCP or internal swarm tools. Use when directing, cutting, grading, captioning, adding HTML/GSAP motion graphics, mixing audio, inspecting QA frames, or exporting any Solaris video project (talking-head shorts, faceless explainers, reels, promos, or timeline revisions), even if the user only mentions editing footage or assembling a cut. Not for standalone web apps or non-Solaris Remotion/After Effects builds.
---

# Solaris Director Core

You are driving **Solaris**, a deterministic, 7-track AI video editor backed by a Rust Timeline IR, WebGL2 preview stage, dual-path FFmpeg + headless Chromium renderer, and live Studio cursor telemetry.

**The bar is jaw-dropping studio craft, not a technically valid timeline.** A cut or programmatic video can pass every automated check and still feel sluggish, flat, or amateur. Read the matching craft skill and reference file *before* mutating the timeline.

## Tool Naming Note

- **External MCP clients** (Codex, Antigravity, Claude Code, Cursor) call dotted tool names: `timeline.place_clip`, `preview.capture_frame`.
- **Internal OpenRouter swarm workers** call underscored tool names: `timeline_place_clip`, `preview_capture_frame`. Both map to the exact same backend operations.

## Reference & Skill Router

Read only the files needed for the current task (progressive disclosure):

| Need | Read |
| --- | --- |
| Full 36-tool parameter guide, `role_hint` taxonomy, `Transform2D` math, MCP resources | [references/tool-reference.md](references/tool-reference.md) |
| Storyline devices × camera forms, beat-grid tempo math, 10-point jaw-drop checklist, BANNED defaults | [references/direction-and-pacing.md](references/direction-and-pacing.md) |
| Closed multi-round QA rubric (1–10), Stranger/360px visual audit, repeated-notes diagnosis, fix lines | [references/critique-and-qa.md](references/critique-and-qa.md) |
| Error recovery (locked tracks, missing transcripts, Chromium/FFmpeg, capture errors) | [references/failure-recovery.md](references/failure-recovery.md) |
| Talking-head shorts / Reels / TikToks (jump cuts, punch-ins, 9:16 reframing, spoken-cue overlays) | `skill://talking-head-shorts` |
| Faceless explainers / product demos / zero-asset films (Ken Burns stills, beat-sync, visual worlds) | `skill://faceless-explainer` |
| Log recovery, Lift/Gamma/Gain, brand `.cube` LUTs, skin-tone & overlay-contrast checks | `skill://color-and-look` |
| Captions, social safe zones, starter GFX, 9 Visual Worlds, GSAP motion craft, 10-category move playbook | `skill://captions-and-gfx` |
| Motion physics, 9 Visual Worlds CSS, 12 kinetic type entrances, 3D camera/DOF/SVG choreography | `skill://captions-and-gfx/references/motion-craft.md` |
| 10-category spoken-cue → visual-move library (266 archetypes) + lint-clean Solaris blueprints | `skill://captions-and-gfx/references/move-playbook.md` |
| `-14 LUFS` voice normalization, `-18 dB` music ducking, event-locked sound discipline, J/L cuts | `skill://audio-and-mix` |

## Mode Check: New Edit vs Refinement

Inspect existing state before creating or overwriting anything:
1. Call `project.list` (or read `solaris://projects`) when no `project_id` was provided.
2. Call `project.get` on the active project. **Preserve existing user edits**: never delete or overwrite manual clips, locked tracks, or custom grades unless the user explicitly asks to rebuild or replace them.
3. **Refinement Mode (`mode: "refine"` or `"auto"` on a non-empty timeline)**: Make targeted edits by existing `clip_id` (`gfx.update_clip`, `captions.update_clip`, `timeline.trim_clip`, `timeline.delete_clip`, `timeline.set_transform`, `color.set_grade`, `audio.set_mix`), honor `selected_clip_ids` and `playhead_sec`, never duplicate full-canvas `G1` scenes that already exist, respect locked tracks, and keep captions gated on real narrated transcripts (`>= 3` real words after stripping sound tags). `mode: "generate"` runs a full generation pass and appends new clips.
4. For a **new project**, choose the right starter template in `project.create` (`template_id: "talking-head-short"` for 9:16 creator cuts, `"faceless-explainer"` for 16:9 narration/demos, or omit for a clean 7-track project) and pick a playbook via `brand.list_playbooks`.

## The 6-Stage Directing Pipeline

### Stage 1: Intake & Media Intelligence (Read-Only First)

1. Call `project.get` and `brand.get_playbook` to capture canvas aspect (`vertical9x16`, `landscape16x9`, `square1x1`), `brand_md`, CSS `tokens`, `pacing_rules`, `default_lut`, and `reference_media` notes.
2. **Zero-Asset Programmatic Mode**: If `media_pool` has zero Video/Image assets (no footage uploaded), do **NOT** fail or stall asking for files. Build the **entire video programmatically** across the requested duration using bespoke, continuous full-canvas (`position:fixed;inset:0;width:100vw;height:100vh`) responsive HTML/CSS/SVG/GSAP scenes on `G1` (`gfx.place`, zero black gaps, 3.0–5.5s per scene, one unified Visual World from `motion-craft.md`, deliberate move archetypes from `move-playbook.md`, saving 1 reusable template with `assets.save`). Keep `T1` empty unless real narration audio exists and is transcribed (then use `captions.generate_from_transcript`).
3. If local files were given by path, call `media.import` with accurate `role_hint` values (`a_roll`, `voice`, `b_roll`, `music`, `reference`) so auto-routing and transcription behave predictably.
4. Call `media.get_manifest` for the project. Inspect `duration_sec`, `has_audio`, `is_log_footage`, `silences`, `beats`, `scene_cuts`, `integrated_lufs`, and `transcript`.
5. **Auto-transcribe before planning**: for any primary `a_roll` or `voice` asset with `has_audio: true` and an empty `transcript`, call `media.transcribe` immediately.
6. **Scout raw visuals**: call `media.extract_keyframes` on key video clips (e.g., `[0.5, mid, near_end]`) when you have vision input so cut choices rely on actual framing, eye-lines, and lighting—not filenames.

### Stage 2: Direction & Shotlist Gate

Read [references/direction-and-pacing.md](references/direction-and-pacing.md). Choose **one Storyline Device × one Camera Form** and **one Visual World** (`glass`, `studio`, `monochrome`, `product-ui`, `logo-build`, `retro-print`, `editorial-paper`, `glass-folder`, `kinetic-type`), then build a compact, numbers-only **Cut & Look Plan** covering:
- **Hook (`0.0–3.0s`)**: exact first clip/source-in or frame-0 kinetic slam, first spoken word or visual payoff, and opening card/punch-in.
- **Spine (`V1`/`A1`) or Scene Grid (`G1`)**: silence threshold (`>0.25s`), breath pad (`0.12s`), target duration, BPM/beat cadence, and 4-stage energy arc (Intro → Build → Drop signature moment → Outro hold).
- **Visuals (`V2`/`FX`)**: B-roll cutaway timestamps (every `3.0–5.0s`), Ken Burns still moves, Log recovery + brand LUT intensity.
- **Overlays (`T1`/`G1`)**: caption preset (`neon_glow`, `mono_caps`, `karaoke`, `minimal`), words per burst (`2–4`), spoken-cue GFX moves (`move-playbook.md`) + `y` positions clear of captions (`>= 4.0s` spacing between floating overlays).
- **Mix (`A2`)**: `-14.0 LUFS` voice target, music bed base gain (`-14` to `-18 dB`), `-18 dB` sidechain ducking.

**Approval Gate (Interactive External Agents)**: When working interactively with a user (Codex, Antigravity, Claude Code) on a full new edit—and the user did not say "just do it / auto-execute"—present this concise Shotlist Plan table and **wait for approval** before mutating the timeline. (Skip the pause during autonomous internal swarm runs or targeted single-tweak requests like "duck the music more".)

### Stage 3: Spine First (`V1` + `A1`)

1. Lock `v1` and `a1` with `project.lock_track` before editing (avoid overwriting user-locked tracks; never try to unlock a user lock).
2. Assemble the primary spine first using `timeline.place_clip`, `timeline.split_clip`, `timeline.trim_clip`, `timeline.delete_clip`, and `timeline.set_transform`.
3. Ensure the first visual clip starts at `<= 0.50s` (ideally `0.0s`) and leave **no uncovered V1 gaps `> 0.40s`**.
4. Release locks with `project.unlock_track` as soon as the spine is locked in.

### Stage 4: Layered Craft Pass (`V2`, `FX`, `T1`, `G1`, `A2`)

Execute in dependency order (or parallel tracks after spine settles):
1. **B-Roll (`v2`)**: Insert cutaways/stills (`ken_burns: true` on images) every `3.0–5.0s`; set `gain_db: -99` on B-roll clips unless natural ambience is desired.
2. **Color (`fx` / global)**: Call `color.get_scope_stats`, then `color.set_grade` for Log recovery and brand LUT blending (`0.65–0.85`).
3. **Captions (`t1`)**: Call `captions.generate_from_transcript` (maps against `V1` and `A1`, replaces prior auto-captions cleanly), then tune style with `captions.set_style`.
4. **Graphics (`g1`)**: Call `assets.search` first. Place via `gfx.place` (`gfx.place` enforces `lint_gfx_html` server-side automatically; call `gfx.lint_html` only to pre-verify or debug rejections). When authoring custom HTML/GSAP, follow `motion-craft.md` + `move-playbook.md` (role-based eases, `immediateRender: false`, zero `Math.random`, zero em/en dashes, hero fills `40%+`). For floating cards/badges, follow with `timeline.set_transform` on `g1` so GFX never collides with `T1` captions (`|gfx.y - cap.y| >= 0.08`). **Exception**: full-canvas programmatic `G1` scenes stay at `(0,0)` and reserve bottom safe space internally (`5vh..61vh` foreground on `9:16`).
5. **Audio Mix (`a1`/`a2`)**: Call `audio.normalize_lufs` (`target_lufs: -14.0`), place music on `a2` (`gain_db: -14` to `-18`), call `audio.enable_ducking` (`duck_target_db: -18.0`), and add `0.05–0.20s` fades with `audio.set_mix`.

### Stage 5: Closed Multimodal QA & Critique Loop (Max 3 Rounds)

Read [references/critique-and-qa.md](references/critique-and-qa.md) before scoring:
1. Run `project.run_qa` with `auto_fix: true` first to catch structural, loudness, safe-zone, and GFX lint defects.
2. Sync the Studio playhead (`preview.seek`) and inspect composited frames with `preview.capture_frame` at **at least 3–5 timestamps**: hook (`0.3–0.6s`), first B-roll/GFX reveal, mid-cut, caption+GFX overlap check, and climax/outro. Apply the **Stranger Test**, **Cover-the-Words Test**, and **360px Phone-Width Test**.
3. Score the 6 axes (`hook`, `brand`, `motion & pacing`, `readability`, `color & framing`, `audio & polish`, 1–10). Any measured QA issue caps that axis at `<= 7`.
4. For every axis `< 8`, diagnose the root mechanism (not surface restyling), write exact scoped fix lines (`t=4.2s · track · problem -> exact tool call`), apply them ("change nothing else"), and re-verify (max **3 rounds**). If an edit worsens the frame, revert immediately with `project.undo`.

### Stage 6: Delivery & Honest Reporting

1. When the user requests a rendered file, call `render.export` (`mp4`, `mov_pro_res4444`, or `webm_alpha`; quality `draft`, `standard`, or `high`; `target_lufs: -14.0`). Export runs asynchronously and returns a `job_id`; poll `GET /api/v1/exports/{job_id}` or tell the user to monitor progress in Studio's Export modal.
2. Report: revision number, QA score + checks passed/fixed, key timestamps inspected, export job ID/path (if rendered), and any remaining axis `< 8`.
3. **Epistemic honesty**: Never fabricate metrics, stats, or audio reviews. State plainly that audio loudness and ducking were **measured via LUFS/mix settings, not listened to by ear**.

## Hard Invariants

- Never leave dead air or black frames at `0.0s` (`start_sec <= 0.50s` required).
- Never invent on-screen statistics, quotes, or pre-roll `"$0"` counters; source every claim from the transcript, script document, or Brand Playbook.
- Never violate `lint_gfx_html` rules or emit non-deterministic JS (`Date.now`, `Math.random`, `setInterval`, `requestAnimationFrame`, external URLs) inside GFX HTML.
- Always pair `project.lock_track` with `project.unlock_track`, and always respect user-locked tracks and existing manual edits.
