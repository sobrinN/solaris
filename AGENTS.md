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
2. **Atomic Mutations Only**: Never mutate `Project` state ad-hoc outside `ProjectEngine::apply(command, meta)`. Every mutation must be undoable, increment `project.revision`, respect track locks, and emit an `EngineEvent` with a semantic `ui_anchor`.
3. **No Node/JS Agent Frameworks**: Agent orchestration lives in native Rust (`crates/agent-runtime` in Phase 3). Do not introduce Node sidecars, Mastra, or Python agent runtimes.
4. **Security & Secrets**:
   - Never store or log `OPENROUTER_API_KEY` or bearer tokens in frontend `localStorage`, URLs, or console logs.
   - Enforce strict path canonicalization/confinement in `media-engine` and `server` when serving or importing local files.
   - Require bearer token auth whenever `solaris-server` binds outside loopback (`127.0.0.1` / `::1`).
5. **FFmpeg Graceful Degradation**: `media-engine` must never panic if `ffmpeg` or `ffprobe` are missing from `PATH`. Report availability cleanly in `/api/v1/health` and honor `SOLARIS_FFMPEG_PATH` / `SOLARIS_FFPROBE_PATH`.

---

## 3. UI & Design System Rules (`DESIGN.md`)

- **Palette Discipline**: Resolve UI surfaces strictly from Solaris Solar Horizon pigments (`Void #080404`, `Carbono #1A0E0E`, `Grafite #1F1F1F`, `Areia #F7F5EF`, `Vermelho Foco #C31818`, `Laranja Criação #F05A00`, `Amarelo Ideias #F7B733`).
- **Typography**: Use bundled `@fontsource-variable/sora` (`300–700`) with `tabular-nums` for timecodes, durations, and LUFS metrics. Do **not** add external Google Fonts `<link>` tags or monospace UI chrome.
- **Motion & Performance**: **Zero `framer-motion`**. Use pure CSS compositor animations (`transform`, `opacity`, `scaleX`) only. Never use `transition-all` or transition layout properties. Always respect `@media (prefers-reduced-motion: reduce)`.
- **Contrast**: Never use `Vermelho Foco` (`#C31818`) as bare body text on dark surfaces (use as fill, dot, or border with `Areia` text). `--accent` is strictly a dark hover surface tint.

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
