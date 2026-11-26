# Solaris Engineering & Agent Guidelines

Read this file before modifying any code inside `video/solaris/`.

---

## 1. Context & Workspace Boundaries

- **Strict Directory Boundary**: All Solaris source, docs, tests, and runtime artifacts live strictly inside `video/solaris/`. **Never** modify, move, or delete `../DESIGN.md`, `../Higgsfield shorts.mp4`, `../hyperframes/`, `../remotion/`, or `../skills-for-reference/`—they are read-only reference materials.
- **Read Context First**: Before making changes, review:
  1. [ARCHITECTURE.md](./ARCHITECTURE.md)
  2. [DESIGN.md](./DESIGN.md)
  3. [docs/TIMELINE_IR.md](./docs/TIMELINE_IR.md)
  4. [tasks/todo.md](./tasks/todo.md) and [tasks/lessons.md](./tasks/lessons.md)
- **Update Progress**: Mark tasks complete in `tasks/todo.md` and record non-obvious bugs, invariants, or architectural decisions in `tasks/lessons.md`.

---

## 2. Architectural Invariants

1. **Transport Boundary**: Never call Tauri `invoke()` directly from React UI components for timeline, media, brand, or asset state. All editor state must go through `packages/engine-client` (`SolarisEngineClient`) talking to `crates/server`.
2. **Atomic Mutations Only**: Never mutate `Project` state ad-hoc outside `ProjectEngine::apply(command, meta)`. Every mutation must be undoable, increment `project.revision`, respect track locks, and emit an `EngineEvent` with a semantic `ui_anchor`. Project deletion (`DELETE /api/v1/projects/{id}`) must cancel active swarm runs, clear project logs, and remove both the project JSON and `uploads/{id}` directory.
3. **No Node/JS Agent Frameworks**: Agent orchestration lives in native Rust (`crates/agent-runtime`). Do not introduce Node sidecars, Mastra, or Python agent runtimes.
4. **Single-Source-of-Truth Skills & Prompt Caching**: `skills/` is embedded via `include_dir!` with `crates/agent-runtime/build.rs` emitting recursive `cargo:rerun-if-changed` directives. Wire new skill references into `skill_references_for_role` (`crates/agent-runtime/src/skills.rs`), keep scene-worker system prompts static across sibling workers for prompt caching, and inject `critique-and-qa.md` into Vision QA.
5. **Security & Secrets**:
   - Never store or log `OPENROUTER_API_KEY` or bearer tokens in frontend `localStorage`, URLs, or console logs.
   - Enforce strict path canonicalization/confinement in `media-engine` and `server` when serving or importing local files.
   - Require bearer token auth whenever `solaris-server` binds outside loopback (`127.0.0.1` / `::1`).
6. **FFmpeg & Chromium Export Pipeline**: `media-engine` must never panic if `ffmpeg`, `ffprobe`, or Chromium are missing from `PATH`. Report availability cleanly in `/api/v1/health`, honor `SOLARIS_FFMPEG_PATH`, `SOLARIS_FFPROBE_PATH`, `SOLARIS_CHROME_PATH`, and `SOLARIS_GFX_WORKERS` (multi-worker parallel Chromium CDP pool with isolated profiles and Z-order preservation).
7. **Web Export Delivery**: On web builds, default to Browser Downloads and use `window.showSaveFilePicker()` deferred until the user starts a render (or clicks Save). Never use `showDirectoryPicker()` (blocked by Chromium on standard user folders) and never open `showSaveFilePicker()` pre-render (which creates empty 0-byte files). Keep the native folder picker for Tauri desktop builds.

---

## 3. UI & Design System Rules (`DESIGN.md`)

- **Palette Discipline**: Resolve UI surfaces strictly from Solaris Solar Horizon pigments (`Void #080404`, `Carbono #1A0E0E`, `Grafite #1F1F1F`, `Areia #F7F5EF`, `Vermelho Foco #C31818`, `Laranja Criação #F05A00`, `Amarelo Ideias #F7B733`).
- **Typography**: Use bundled `@fontsource-variable/sora` (`300–700`) with `tabular-nums` for timecodes, durations, and LUFS metrics. Do **not** add external Google Fonts `<link>` tags or monospace UI chrome.
- **Motion & Performance**: **Zero `framer-motion`**. Use pure CSS compositor animations (`transform`, `opacity`, `scaleX`) only. Never use `transition-all` or transition layout properties. Always respect `@media (prefers-reduced-motion: reduce)`.
- **Contrast**: Never use `Vermelho Foco` (`#C31818`) as bare body text on dark surfaces (use as fill, dot, or border with `Areia` text). `--accent` is strictly a dark hover surface tint.
- **Aspect-Aware Studio Layout & Director Panel**: Keep the Program Monitor column dynamically sized to the project's aspect ratio (`9:16`, `1:1`, `16:9`) so portrait/square canvases don't waste horizontal space on empty black bars. Keep the Director & Swarm panel streamlined with a single `Enhance` (QA auto-fix) header button and automatic refine-in-place behavior when timeline clips exist (no `Fresh cut` toggle—users create a new project to start from zero).

---

## 4. Tech-Specific Guardrails

- **Axum 0.8**: Always use `{id}` and `{media_id}` route parameter syntax—never `:id` (which panics at router initialization).
- **Rust Edition 2024**: Avoid `unsafe` blocks (note that `std::env::set_var` is `unsafe` in Rust 2024; pass config structs in tests instead).
- **Keep Files Modular**: Keep source files focused and under ~500 lines where practical.

---

## 5. Mandatory Verification Before Completion

Run all checks from `video/solaris/` after your final edit:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm run typecheck
npm run build
```
