# MCP Server & Unified Skills Architecture

Solaris treats **MCP** and **Skills** as two complementary halves of agentic video editing:
- **MCP = The Hands**: Strongly-typed tools + live event bridge so internal swarm workers AND external agents (Codex, Antigravity, Claude Code, Cursor) mutate the exact same Rust Timeline IR.
- **Skills = The Craft**: Procedural directing, pacing, color, captioning, programmable GFX, audio mixing, and closed-loop QA workflows distilled into progressive-disclosure skill packages (`SKILL.md` + `references/*.md` + `agents/openai.yaml`).

---

## 1. Canonical MCP Tool Surface (36 Tools)

Transport: `stdio` (CLI coding agents) + `Streamable HTTP` (`/mcp`) with bearer token auth, `Origin`/`Host` validation, `Content-Type: application/json`, and tracked `Mcp-Session-Id` lifecycle (`POST` / `DELETE`).

| Group | Tools |
| --- | --- |
| `project.*` | `project.list`, `project.create` (supports `template_id`), `project.get`, `project.run_qa`, `project.undo`, `project.redo`, `project.lock_track`, `project.unlock_track` |
| `media.*` | `media.import`, `media.get_manifest` (silences, beats, scene cuts, peaks, LUFS, transcript), `media.extract_keyframes`, `media.transcribe` |
| `timeline.*` | `timeline.place_clip`, `timeline.split_clip`, `timeline.trim_clip`, `timeline.delete_clip`, `timeline.set_transform` |
| `color.*` | `color.set_grade` (Lift/Gamma/Gain, temp, tint, contrast, sat, exp, LUT), `color.get_scope_stats` (grade/LUT/log metadata; pair with `preview.capture_frame` for visual verification) |
| `captions.*` | `captions.generate_from_transcript`, `captions.place_script`, `captions.set_style`, `captions.update_clip` |
| `gfx.*` & `assets.*` | `assets.search`, `assets.save`, `gfx.place`, `gfx.update_clip`, `gfx.lint_html` |
| `audio.*` | `audio.set_mix`, `audio.normalize_lufs`, `audio.enable_ducking` |
| `brand.*` | `brand.get_playbook`, `brand.list_playbooks` |
| `preview.*` & `render.*` | `preview.seek`, `preview.capture_frame` (returns composited/graded PNG for multimodal QA), `render.export` |

Every external MCP tool call passes `AgentRole::McpExternal` into `ProjectEngine::apply(...)`, automatically animating the external agent's cursor inside the Studio UI in real time.

### Refinement Mode Request Fields (`POST /api/v1/projects/{id}/swarm/run`)

REST and Studio callers can target existing cuts without regenerating the timeline:
- `mode`: `"auto" | "generate" | "refine"` (default `"auto"`, which resolves to `"refine"` when the project already has timeline clips; `"refine"` on an empty timeline returns HTTP `400`).
- `selected_clip_ids`: optional `string[]` (max 64 safe clip IDs that exist on the project) passed as focused edit context.
- `playhead_sec`: optional finite `number >= 0` indicating the current playhead timestamp.
- `conversation_history`: optional `{ role: "user" | "assistant", text: string }[]` (last 12 turns, `<= 2000` chars each).
- Responses expose `refinement_summary: string | null` on `SwarmRunStatus`.

---

## 2. Progressive-Disclosure `skills/` Pack

Skills live in `solaris/skills/`, are compiled into the Rust binary via `include_dir!`, exposed via MCP Resources (`skill://<name>` and `skill://<name>/references/<file>.md`), and recursively installed into external coding agents via `solaris-mcp install-skills --target antigravity|codex|claude|cursor`:

1. **`director-core`** (`SKILL.md` + 4 references: `tool-reference.md`, `direction-and-pacing.md`, `critique-and-qa.md`, `failure-recovery.md`): Master workflow router, intake & approval gate, numbers-only shotlist plan, 7-track execution order, closed multi-round visual QA rubric (max 3 rounds), rollback via `project.undo`, and honest delivery disclosure.
2. **`talking-head-shorts`**: 0–3s hook framing, breath-safe jump-cut math (`>0.25s` silences with `0.12s` breath pads, contiguous placement), `1.00`↔`1.08–1.14` punch-in zooms, smart `9:16` eye-line framing, and `V2` B-roll J/L cutaways every `3–5s`.
3. **`faceless-explainer`**: Under-`3.5s` visual density contract, `A1` voiceover spine, mandatory `ken_burns: true` on stills, beat-synced cutting (`±0.10s`), structured starter GFX hierarchy, and caption alignment.
4. **`color-and-look`**: Honest `color.get_scope_stats` metadata check paired with `preview.capture_frame` visual verification, Log-to-Rec.709 numerical recovery table, global vs existing `fx` clip rules, brand `.cube` LUT blending (`0.65–0.85`), and skin-tone discipline.
5. **`captions-and-gfx`** (`SKILL.md` + 2 references: `gfx-contract.md`, `starter-assets.md`): Social `9:16`/`16:9` safe zones, all 4 caption presets (`neon_glow`, `karaoke`, `mono_caps`, `minimal`), searching all 5 starter assets first, mandatory post-place `timeline.set_transform` offset on `g1`, and the full deterministic offline HTML/CSS/GSAP seekable sandbox contract (`window.gsap`, bundled `Sora` + Brand `@font-face`, 4 seek hooks, `{{prop}}` escaping, 128 KB limit).
6. **`audio-and-mix`**: Voice-first `-14.0 LUFS` normalization (respecting the `±12 dB` safety clamp), `A2` music beds (`-14` to `-18 dB` base gain) with `-18.0 dB` FFmpeg sidechain ducking, click-free `0.05–0.20s` fades, muted B-roll scratch audio, and honest "measured via LUFS, not listened to" reporting.
