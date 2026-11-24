# Solaris Development Roadmap

## Phase 1 — Foundation & Canonical Engine (Complete)
- [x] Project governance & documentation (`README.md`, `ARCHITECTURE.md`, `DESIGN.md`, `AGENTS.md`, `docs/*`)
- [x] `crates/core-ir`: Canonical 7-track Timeline IR, atomic commands, undo/redo, track locks, Brand Playbook & Saved Code Asset schemas
- [x] `crates/media-engine`: `AssetStorage` trait (`LocalFsStorage`), FFmpeg/FFprobe probing, log detection, silence detection, LUFS loudness, waveform peak extraction, proxy generation
- [x] `crates/server`: `axum` 0.8 HTTP + WebSocket daemon, range media streaming, background media analysis, workspace persistence
- [x] `packages/engine-client` + `apps/studio-web`: Solar Horizon Pre-Editor Chat + 6-Panel Live NLE Studio + Hybrid Swarm Cursors + Brand Playbook Drawer

---

## Phase 2 — Tauri v2 Desktop, WebGL2 Scopes, GFX Runtime & Dual-Path Export (Complete)
- [x] Phase 1 review fixes: persistent `activity_log` hydration, spread-out Swarm Cursors, Recent Projects, constant-memory `ReaderStream` media streaming, macOS Homebrew FFmpeg + Chromium auto-discovery
- [x] Brand Playbook Reference Media & Media Kit (`POST/PATCH/DELETE /api/v1/brands/{id}/media`, `.cube` 3D LUT parsing & `/lut` JSON endpoint)
- [x] Thin Tauri v2 desktop wrapper (`apps/desktop/src-tauri`) embedding `solaris_server::spawn_embedded` on `127.0.0.1:0` with session bearer token + `engineBridge.ts`
- [x] Real-time WebGL2 shader canvas (Lift/Gamma/Gain, Exposure, Contrast, Saturation, Temp, Tint, `.cube` 3D LUT) + authentic Rec.709 Luma Waveform & Cb/Cr Vectorscope
- [x] Sandboxed HTML/GSAP GFX preview runtime (`packages/gfx-runtime`) with `--brand-*` CSS token injection, `postMessage` seek, and typed JSON Schema Inspector controls
- [x] Dual-path FFmpeg export filtergraph (`crates/media-engine/src/export.rs`) supporting MP4, ProRes 4444 MOV Alpha, WebM VP9 Alpha, `-14 LUFS` `loudnorm`, and `sidechaincompress` music ducking + `ExportModal`

---

## Phase 3 — OpenRouter Multimodal Swarm, MCP Server & Skills (Complete)
- [x] Native Rust OpenRouter client (`crates/agent-runtime`) with live Model Advisor (`GET /api/v1/models` modality inspection), SSE streaming, and fallback routing
- [x] Director + 6-role parallel Swarm (`razor`, `broll`, `grade`, `caps`, `gfx`, `mix`) executing against `ProjectEngine` track locks
- [x] Local FFmpeg 8 `whisper` filter / OpenRouter STT word-level transcription pipeline (`crates/media-engine/src/transcribe.rs`)
- [x] Built-in `crates/mcp-server` (`stdio` + Streamable HTTP `/mcp`) + `skills/` pack (`include_dir!` + `skill://*` MCP resources + `solaris-mcp install-skills`)

---

## Phase 4 — Multimodal QA Loop, Templates & OSS Release (Complete)
- [x] Automatic Stage-3 Playback QA pass (`run_qa_pass` + `POST /api/v1/projects/{id}/qa` + `project.run_qa` MCP tool: `preview.capture_frame` hook/mid/climax inspection, Log-to-Rec.709 recovery, `-14 LUFS` & `-18 dB` ducking checks, `9:16` safe-zone & GFX overlap fixes, and deterministic HTML lint)
- [x] Starter Talking-Head & Faceless Explainer templates (`GET /api/v1/templates`, `template_id` on project creation, `solaris://templates` MCP resource) + community Code-Asset registry (`solaris.asset-pack/v1` import/export endpoints, `solaris://assets/pack` MCP resource, and Studio UI Import/Export)
- [x] Dockerized `solaris-server` (`Dockerfile`, `.dockerignore`, `docker-compose.yml`, `.env.example`) + AWS SigV4 `S3Storage` backend adapter (`crates/media-engine/src/s3_storage.rs`) for optional Web SaaS / R2 / MinIO deployment
