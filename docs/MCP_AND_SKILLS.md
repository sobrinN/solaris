# MCP Server & Unified Skills Architecture

Solaris treats **MCP** and **Skills** as two complementary halves of agentic video editing:
- **MCP = The Hands**: Strongly-typed tools + live event bridge so internal swarm workers AND external agents (Codex, Antigravity, Claude Code) mutate the exact same Rust Timeline IR.
- **Skills = The Craft**: Procedural directing, pacing, color, captioning, and audio mixing knowledge distilled from professional editing workflows so agents don't produce amateur edits.

---

## 1. Canonical MCP Tool Surface (33 Tools)

Transport: `stdio` (CLI coding agents) + `Streamable HTTP` (`/mcp`) with bearer token auth, `Origin`/`Host` validation, `Content-Type: application/json`, and tracked `Mcp-Session-Id` lifecycle (`POST` / `DELETE`).

| Group | Tools |
| --- | --- |
| `project.*` | `project.list`, `project.create` (supports `template_id`), `project.get`, `project.run_qa`, `project.undo`, `project.redo`, `project.lock_track`, `project.unlock_track` |
| `media.*` | `media.import`, `media.get_manifest` (silences, beats, scene cuts, peaks, LUFS, transcript), `media.extract_keyframes`, `media.transcribe` |
| `timeline.*` | `timeline.place_clip`, `timeline.split_clip`, `timeline.trim_clip`, `timeline.delete_clip`, `timeline.set_transform` |
| `color.*` | `color.set_grade` (Lift/Gamma/Gain, temp, tint, contrast, sat, exp, LUT), `color.get_scope_stats` |
| `captions.*` | `captions.generate_from_transcript`, `captions.set_style` |
| `gfx.*` & `assets.*` | `assets.search`, `assets.save`, `gfx.place`, `gfx.lint_html` |
| `audio.*` | `audio.set_mix`, `audio.normalize_lufs`, `audio.enable_ducking` |
| `brand.*` | `brand.get_playbook`, `brand.list_playbooks` |
| `preview.*` & `render.*` | `preview.seek`, `preview.capture_frame` (returns graded PNG for multimodal QA), `render.export` |

Every external MCP tool call passes `AgentRole::McpExternal` into `ProjectEngine::apply(...)`, automatically animating the external agent's cursor inside the Studio UI in real time.

---

## 2. Single-Source-of-Truth `skills/` Pack

Skills live in `solaris/skills/`, are compiled into the Rust binary via `include_dir!`, exposed via MCP Resources (`skill://<name>`), and installable into external coding agents via CLI:

1. **`director-core`**: Inspect Media Intelligence Manifest → build Shotlist/Hook Plan → execute track-ordered edits (`V1/A1` spine first, parallel `T1/G1/V2/FX/A2` fan-out) → run Multimodal QA (`preview.capture_frame` + LUFS audit).
2. **`talking-head-shorts`**: 0–3s hook framing, silence/breath cuts with `0.12s` breath-safe pads, `104–115%` punch-in zooms on emphasis beats, smart `9:16` face reframing, B-roll cutaways every `3–5s`.
3. **`faceless-explainer`**: Scene-by-scene visual pacing, kinetic typography, animated charts/callout cards, Ken Burns stills, SFX accents on visual reveals.
4. **`color-and-look`**: Reading Luma Waveform & Vectorscope telemetry, Log-to-Rec.709 recovery, Lift/Gamma/Gain balance, `.cube` LUT blending, protecting skin tones and highlights.
5. **`captions-and-gfx`**: Social UI safe zones (avoiding TikTok/Reels bottom & right overlays), 2–4 word bursts, searching `assets.search` before writing new HTML/GSAP code, and the seekable component contract (`data-start`, `data-duration`, `--brand-*` CSS variables, zero `Date.now()`).
6. **`audio-and-mix`**: Voice cleanup, `-14 LUFS` integrated loudness target, voice-driven sidechain music ducking (`-16` to `-20 dB`), J/L cuts, and subtle whoosh/riser placement.
