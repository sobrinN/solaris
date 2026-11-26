# Solaris

**The open-source, local-first AI video editor.**  
Prompt a video in chat, attach raw footage and references, apply your brand playbook, and watch an autonomous multimodal swarm—or your own coding agent over MCP—edit the entire timeline programmatically.

---

## Core Capabilities

- **Chat-First Intake & Project Lifecycle**: Start in an obsidian **MediaComposer** chat—attach A-roll, B-roll, voiceovers, music, and scripts, choose a Brand Playbook and aspect ratio (`9:16`, `16:9`, `1:1`), and transition straight into the live NLE studio. Switch, rename, create blank projects, or delete projects with inline confirmation from both the Studio header (`ProjectManagerPopover`) and the home screen's Recent dropdown.
- **Aspect-Aware Live NLE & Multi-Selection**: Media-first studio layout with a 3-zone header, left **Director & Swarm** panel (`Prompt | Activity | Logs` + one-click `Enhance` QA auto-fix), center **Program / Source Monitor** that dynamically adapts its column width to `9:16`, `1:1`, or `16:9` (eliminating wasted black sidebars on portrait/square videos), toggleable **Broadcast Scopes** (Luma Waveform, Vectorscope, `-14 LUFS` loudness), segmented **Color / Captions / Clip Inspector**, and a 7-track semantic timeline (`T1 TEXT`, `G1 GFX`, `V2 B-ROLL`, `V1 A-ROLL`, `FX GRADE`, `A1 VOICE`, `A2 MUSIC`) with Media/GFX library drawer, Ctrl/Cmd-click, Shift-range, marquee selection, group drag, and single-undo batch edits.
- **In-Studio Director Refinement & Narration-Only Captions**: Prompts automatically refine existing cuts in place using selected clips, playhead context, quick refinement chips, and per-project history—or generate from scratch on blank projects. Captions are generated strictly from real narrated speech transcripts (`>= 3` spoken words after stripping sound tags) and stay in sync.
- **OpenRouter BYOK + Model Advisor**: Bring your own OpenRouter API key. Solaris inspects live model modalities (`video`, `audio`, `image`, `text`, `tools`) and recommends true omni-modal models for best results, with automatic keyframe + transcript fallbacks for vision/text models.
- **Unified MCP + Skills**: Connect Codex, Antigravity, Gemini, Claude Code, or Cursor directly to the live NLE via MCP (`stdio` + Streamable HTTP) backed by a canonical `skills/` pack (`director-core`, `talking-head-shorts`, `faceless-explainer`, `color-and-look`, `captions-and-gfx`, `audio-and-mix`) with progressive-disclosure craft references (`motion-craft.md`, `move-playbook.md`, `gfx-contract.md`, `critique-and-qa.md`).
- **Brand Playbooks & Reusable Code Assets**: Store `BRAND.md` directing rules, CSS design tokens, custom fonts, and `.cube` LUTs. Save any LLM-authored HTML/GSAP motion graphic with a typed JSON Schema so future videos reuse it in milliseconds.
- **Hybrid Dual-Path Engine & Parallel Chromium Pool**: Native Rust + FFmpeg 8/9 + WebGL2 handles footage, grading, reframing, and audio mixing; zero-build HTML/GSAP overlays render with true RGBA alpha via a parallel multi-worker Chromium CDP pool (`SOLARIS_GFX_WORKERS`) with live diagnostic log milestones.
- **Local-First Desktop & Web-Ready**: Runs as a lightweight local Rust daemon (`axum` + `tokio`) with Tauri v2 desktop packaging (native folder picker) and web export delivery (default Browser Downloads + deferred `Save As` picker via `showSaveFilePicker`).

---

## Tech Stack

| Layer | Technology |
| --- | --- |
| **Core Engine & Server** | Rust (`axum`, `tokio`, `serde`, `thiserror`, `tracing`) |
| **Media Pipeline** | FFmpeg / FFprobe + parallel Headless Chromium CDP pool (`whisper`, `ebur128`, `silencedetect`, `lut3d`, `sidechaincompress`) |
| **Frontend UI** | React 19, TypeScript, Vite, Tailwind CSS v4, `@fontsource-variable/sora`, pure CSS compositor motion |
| **Desktop Shell** | Tauri v2 |
| **Agent Protocol** | OpenRouter REST/SSE (Rust) + MCP (`stdio` / Streamable HTTP) |

---

## Directory Layout

```text
solaris/
├── Cargo.toml                 # Rust workspace
├── package.json               # npm workspaces (engine-client + studio-web)
├── ARCHITECTURE.md            # 4-layer architecture & transport boundary
├── DESIGN.md                  # Solaris Solar Horizon design system
├── AGENTS.md                  # Engineering & agent rules
├── docs/
│   ├── ASSET_PACK_SPEC.md     # solaris.asset-pack/v1 community GFX exchange spec
│   ├── TIMELINE_IR.md         # Canonical Timeline IR, commands & WebSocket events
│   ├── MCP_AND_SKILLS.md      # MCP tools, resources & skills pack spec
│   └── ROADMAP.md             # 4-phase delivery roadmap
├── tasks/
│   ├── todo.md                # Active phase checklist
│   └── lessons.md             # Architectural decisions & lessons learned
├── crates/
│   ├── core-ir/               # Timeline IR, undo/redo, track locks, brands, assets
│   ├── media-engine/          # AssetStorage trait, FFmpeg/FFprobe probing, LUTs, STT & export
│   ├── agent-runtime/         # OpenRouter SSE client, ModelAdvisor, 36-tool runtime & Swarm
│   ├── mcp-server/            # Built-in MCP server (stdio + Streamable HTTP /mcp) & solaris-mcp CLI
│   └── server/
│       └── src/routes/        # Modular axum HTTP/WS/MCP routes (shared by Desktop & Web)
├── skills/                    # 6 embedded domain editing skills (include_dir! + skill://*)
├── packages/
│   ├── engine-client/         # Typed TS HTTP + WebSocket client
│   └── gfx-runtime/           # Deterministic seekable HTML/GSAP runtime & schema extractor
└── apps/
    ├── desktop/src-tauri/     # Thin Tauri v2 desktop wrapper embedding solaris-server
    └── studio-web/            # Pre-Editor Chat + Aspect-Aware NLE + ProjectManagerPopover + WebGL2 Scopes + Swarm Cursors
```

---

## Local Development Quick Start

### Prerequisites
- **Rust** `1.85+` (edition 2024)
- **Node.js** `22+` & **npm**
- **FFmpeg & FFprobe** (either on `PATH` or pointed to via `SOLARIS_FFMPEG_PATH` and `SOLARIS_FFPROBE_PATH`)

### 1. Start the Rust Core Server
```bash
cd solaris
cargo run -p solaris-server
# Listens on http://127.0.0.1:41731 by default
```

If FFmpeg is not on your global `PATH`, pass custom binary paths:
```bash
SOLARIS_FFMPEG_PATH=/path/to/ffmpeg \
SOLARIS_FFPROBE_PATH=/path/to/ffprobe \
OPENROUTER_API_KEY=sk-or-v1-... \
cargo run -p solaris-server
```

Optional agent, rendering & storage environment variables:
- `OPENROUTER_API_KEY` — enables OpenRouter multimodal Director + Specialist Swarm & OpenRouter STT (omit for deterministic local fallback mode)
- `SOLARIS_OPENROUTER_MODEL` — override the Model Advisor's primary OpenRouter model
- `SOLARIS_STT_MODEL` — override the OpenRouter Speech-to-Text model
- `SOLARIS_WHISPER_MODEL` — path to a local GGML Whisper model for FFmpeg 8+ `whisper` filter transcription
- `SOLARIS_GFX_WORKERS` — number of parallel Chromium CDP workers for overlay rendering (`1–8`, defaults to CPU count capped at `4`)
- `SOLARIS_STORAGE_BACKEND` — set to `s3` (with `SOLARIS_S3_BUCKET`, `SOLARIS_S3_ACCESS_KEY_ID`, `SOLARIS_S3_SECRET_ACCESS_KEY`, optional `SOLARIS_S3_ENDPOINT` / `SOLARIS_S3_REGION`) for S3/R2/MinIO object storage

### 2. Docker Deployment (Self-Hosted / Web SaaS)
```bash
cd solaris
cp .env.example .env
SOLARIS_AUTH_TOKEN=replace-with-strong-token docker compose up --build
# Optional local S3-compatible MinIO:
# SOLARIS_AUTH_TOKEN=replace-with-strong-token docker compose --profile s3-local up --build
```

### 3. Connect External Coding Agents via MCP (`solaris-mcp`)
```bash
cd solaris
# Install the 6 Solaris editing skills into Codex, Antigravity, Gemini, Claude Code, or Cursor
cargo run -p solaris-mcp-server -- install-skills --target codex

# Run the stdio MCP server (bridges to http://127.0.0.1:41731/mcp by default)
cargo run -p solaris-mcp-server -- serve
```

### 4. Install Frontend Dependencies & Run Studio UI
```bash
cd solaris
npm install
npm run dev
# Opens Vite dev server on http://localhost:5173
```

### 5. Run Verification Checks
```bash
# Rust
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# TypeScript & Frontend
npm run typecheck
npm run build
```

---

## License

Apache License 2.0 — see [LICENSE](./LICENSE).
