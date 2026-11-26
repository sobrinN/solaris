# Solaris Architecture

**Version:** 1.4.0  
**Status:** All 4 Phases Complete (Foundation, Desktop/Export, OpenRouter Swarm & MCP, Multimodal QA & Cloud/Docker Release)

---

## 1. Design Philosophy

Solaris is built around three non-negotiable engineering goals:

1. **Ultra-Lightweight Native Core**: No Electron, no Node.js agent sidecars (e.g., no Mastra). All timeline state, media intelligence, OpenRouter orchestration, and MCP serving run in a single compiled Rust binary (~15–25 MB) paired with a thin Tauri v2 desktop shell.
2. **Dual-Path Hybrid Rendering**: Raw camera footage, smart `9:16` reframing, 3-way color grading, `.cube` LUTs, and broadcast audio mixing stay 100% native in **Rust + FFmpeg + GPU**. A multi-worker **Headless Chromium CDP pool** (`SOLARIS_GFX_WORKERS`) is invoked **only** for time ranges containing custom HTML/CSS/GSAP/WebGL motion graphics (`G1 GFX` / `T1 TEXT`), rendering isolated RGBA alpha frames in parallel with Z-order preservation and monotonic progress into FFmpeg's `overlay` filtergraph.
3. **Desktop-First, Web-Ready Transport Boundary**: The React UI **never** calls desktop-only IPC (`tauri::invoke`) for editor logic. Every mutation, query, media stream, and live cursor event flows through `packages/engine-client` over HTTP + WebSocket to `crates/server`.

---

## 2. Four-Layer System Architecture

```mermaid
flowchart TD
    subgraph UI["Layer 1: Experience & Studio UI (React 19 + Vite + Tailwind v4)"]
        Chat["Pre-Editor Chat + MediaComposer<br/>(Prompt, Attachments, Brand Selector, Aspect Toggle, Recent Projects)"]
        NLE["Aspect-Aware Live NLE Studio<br/>(Director & Swarm, Adaptive Program/Source, Toggleable Scopes, Segmented Inspector, 7 Tracks)"]
        Cursors["Hybrid Swarm Cursor Projector<br/>(Pure CSS translate3d, Zero framer-motion)"]
        Preview["Dual-Layer Preview Stage<br/>(WebGL2 Video + Grade Canvas + Sandboxed HTML/GSAP Layer)"]
    end

    subgraph Agents["Layer 2: Agentic Brain & MCP Gateway (Rust Native)"]
        OR["OpenRouter Orchestrator (Rust reqwest + SSE)<br/>Multimodal Director + 6-Role Specialist Swarm + Vision QA"]
        Advisor["OpenRouter Model Advisor<br/>Checks video+audio+image+text+tools modalities"]
        MCP["Built-in MCP Server (stdio + Streamable HTTP)<br/>Connects Codex, Antigravity, Gemini, Claude Code, Cursor"]
        Skills["Single-Source-of-Truth skills/ Pack<br/>Compiled via include_dir! + build.rs + MCP Resources"]
    end

    subgraph Core["Layer 3: Rust Core Engine"]
        IR["Canonical Timeline IR & Op-Log (core-ir)<br/>Transactions, Undo/Redo, Per-Track Locks"]
        Intel["Deterministic Media Intelligence (media-engine)<br/>Probes, Proxies, Waveforms, Silences, Loudness, Whisper"]
        Bus["Real-Time Event Bus (axum WebSocket)"]
    end

    subgraph Render["Layer 4: Dual-Path Render Pipeline"]
        Native["Native Path: Rust + FFmpeg 8/9<br/>(A-Roll, B-Roll, Reframe, Color/LUT, -14 LUFS Mix & Ducking)"]
        Overlay["Programmable GFX Path: Parallel Chromium CDP Pool<br/>(Isolated multi-worker RGBA capture ONLY for G1/T1 frames)"]
        Mux["Final Compositor & Hardware Encoder<br/>(VideoToolbox / NVENC / libx264 -> MP4/MOV/WebM)"]
    end

    Chat -->|"Brief + Assets + Brand"| OR
    Chat -->|"Transition to Studio"| NLE
    OR <-->|"Atomic TimelineCommands"| IR
    Advisor --> OR
    MCP <-->|"Same Tool Contract"| IR
    Skills --> OR
    Skills --> MCP
    IR --> Bus
    Bus --> NLE
    Bus --> Cursors
    Bus --> Preview
    Intel -->|"MediaIntelligenceManifest"| OR
    IR --> Native
    IR --> Overlay
    Native --> Mux
    Overlay -->|"RGBA Alpha Stream"| Mux
```

---

## 3. The Transport Boundary (Desktop ↔ Web SaaS Portability)

| Concern | Local-First Desktop Mode | Future Web SaaS Mode |
| --- | --- | --- |
| **UI Transport** | `SolarisEngineClient` → `http://127.0.0.1:<port>` + `ws://` | `SolarisEngineClient` → `https://api...` + `wss://` |
| **Asset Storage (`AssetStorage` trait)** | `LocalFsStorage` (zero-copy local disk + range streaming) | `S3Storage` / `R2Storage` (signed URLs / object store) |
| **FFmpeg & Render** | Local `MediaToolchain` (VideoToolbox / NVENC) | Containerized Rust/FFmpeg worker pool |
| **Secrets** | OS Keychain via Tauri v2 (never in React state) | Server-side encrypted vault / session |

Because `apps/studio-web` only depends on `packages/engine-client`, **~95% of both the Rust backend and TypeScript frontend remain identical** between Desktop and Cloud deployments.

---

## 4. Workspace Crate & Package Map

- **`crates/core-ir`**: Pure Rust domain crate. Owns `Project`, 7 semantic `Track`s, `Clip` variants, `ColorGrade`, `BrandPlaybook`, `SavedCodeAsset`, `CodeAssetPackManifest` (`solaris.asset-pack/v1`), `ProjectTemplateInfo` (`talking-head-short`, `faceless-explainer`), `QaReport`, `SwarmRunStatus`, atomic `TimelineCommand`s, `ProjectEngine` (with undo/redo stacks & per-track concurrency locks), and atomic JSON persistence.
- **`crates/media-engine`**: Owns `AssetStorage` trait (`LocalFsStorage` & AWS SigV4 `S3Storage`), `MediaToolchain` capability detection, `ffprobe` JSON parsing, log-footage detection, `silencedetect` + `ebur128` LUFS parsing, raw `f32le` PCM waveform peak extraction, proxy generation, Whisper STT parsing (`transcribe.rs`), PNG frame capture (`frames.rs`), and dual-path FFmpeg + parallel multi-worker Chromium CDP export (`export.rs`, `gfx_render.rs`).
- **`crates/agent-runtime`**: Native Rust OpenRouter SSE client (`openrouter.rs`), live `ModelAdvisor` (`advisor.rs`), OpenRouter STT (`stt.rs`), 36-tool `SolarisBackend` executor (`tools/` — including `html`/`html_source` aliases on `gfx.place` / `gfx.update_clip`), embedded `skills/` (`include_dir!` + `build.rs` watch + `skill_references_for_role`), and Director + 6-role parallel Swarm & cached Vision QA (`swarm/`).
- **`crates/mcp-server`**: Built-in MCP server (`stdio` HTTP bridge + Streamable HTTP `/mcp`) exposing Solaris tools, `skill://*` / `solaris://*` resources, `initialize` skill instructions, prompts, and the `solaris-mcp` CLI (`serve`, `list-skills`, `install-skills --target codex|antigravity|gemini|claude|cursor`).
- **`crates/server`**: `axum` 0.8 HTTP + WebSocket daemon (`lib` + `bin`). Exposes `/api/v1/*` REST routes (including `PATCH` & `DELETE /api/v1/projects/{id}` with swarm cancellation and upload cleanup), `/mcp` Streamable HTTP, `/api/v1/ws` live event stream, export lifecycle/milestone diagnostic logs, background media analysis tasks, and workspace persistence. Embedded directly by Tauri.
- **`packages/engine-client`**: Zero-dependency TypeScript SDK mirroring `core-ir` contracts and wrapping REST (including `deleteProject`) + auto-reconnecting WebSocket subscriptions.
- **`apps/studio-web`**: React 19 + Vite + Tailwind v4 implementation of the **Solaris Solar Horizon** UI (`PreEditorChat`, `StudioWorkspace` with aspect-aware Program Monitor sizing, `ProjectManagerPopover`, streamlined `SwarmActivityPanel`, segmented `InspectorPanel`, toggleable `BroadcastScopes`, deferred `showSaveFilePicker` `ExportModal`, `SwarmCursorLayer`, and `BrandPlaybookDrawer`).
