# Solaris — Master Hardening & Vision Completion Checklist

> **Context for the implementing agent:**
> Phases 1–4 built Solaris's full foundation and pass all current static checks (`cargo fmt`, `cargo clippy`, 44 Rust tests, Tauri check, TS typecheck/build). However, a full code review against our original product vision (`tasks/back and forth.md`) and docs uncovered **critical runtime bugs, security boundaries, and unimplemented core vision features**.
>
> **Rules:**
> 1. Read `AGENTS.md`, `ARCHITECTURE.md`, `DESIGN.md`, `docs/*`, `tasks/lessons.md`, and `tasks/back and forth.md` before coding.
> 2. Strictly stay inside `video/solaris/` — never touch `../DESIGN.md`, `../hyperframes/`, `../remotion/`, `../skills-for-reference/`, or `../Higgsfield shorts.mp4`.
> 3. Root-cause fixes only — no temporary workarounds, no `// TODO` stubs, no `framer-motion`, no `transition-all`, no `localStorage` secrets, no direct Tauri `invoke()` for editor state.
> 4. Check off each item as completed, record non-obvious gotchas in `tasks/lessons.md`, and run the full verification suite (`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml`, `npm run typecheck`, `npm test`, `npm run build`) plus real FFmpeg/server smoke tests before declaring done.

---

## Track 1 — Security, Sandbox & Deployment Blockers (Fix First)

### 1.1 Docker & S3 Deployment Fixes
- [x] **Fix Docker volume permission crash loop (`Dockerfile:33-45`, `docker-compose.yml:28-29`)**: Run `mkdir -p /var/lib/solaris/data && chown -R solaris:solaris /var/lib/solaris` *before* `USER solaris` so newly created named volumes mount writable instead of `root:root` (`EACCES`).
- [x] **Fix Headless Chromium in Docker (`Dockerfile:22-31`, `crates/media-engine/src/gfx_render.rs:189-210`)**: In `Dockerfile`'s `INSTALL_CHROMIUM=true` branch, install fonts (`fonts-liberation`, `fonts-dejavu-core`); in `gfx_render.rs`, add `--disable-dev-shm-usage` and container-safe `--no-sandbox` (configurable via env / auto on Linux container), add a subprocess timeout, and log stderr on failure instead of silently dropping overlays.
- [x] **Fix `.dockerignore` & `docker-compose.yml` hygiene**: Update `.dockerignore` to exclude `**/node_modules`, `.env.*`, and `.git`; expose `SOLARIS_S3_SESSION_TOKEN`, `SOLARIS_ALLOW_EXTERNAL_READS`, and `SOLARIS_CHROME_PATH` in `docker-compose.yml`; bind MinIO to `127.0.0.1` by default and document `http://minio:9000` for Docker Compose in `.env.example`.
- [x] **Fix Docker / Web static frontend origin & authentication (`apps/studio-web/src/lib/engineBridge.ts:35-37`, `packages/engine-client/src/client.ts:434-435`, `apps/studio-web/src/App.tsx`, `crates/server/src/routes/mod.rs:180-200`)**:
  - Default `baseUrl` in `engineBridge.ts` to `window.location.origin` in production browser builds (keeping `http://127.0.0.1:41731` fallback only for Vite dev server / explicit `VITE_SOLARIS_ENGINE_URL`).
  - Resolve relative WS URLs cleanly against `window.location` in `client.ts`.
  - Add a clean Solar Horizon **Engine Sign-In Modal** in `studio-web` when the server returns `401 Unauthorized` (calling `POST /api/v1/auth/session` to set the `HttpOnly` session cookie + ephemeral `media_ticket`, never persisting tokens in `localStorage`).

### 1.2 Export Path Confinement, GFX Sandbox & HTTP/WS/MCP Security
- [x] **Resolve `media_pool` paths before FFmpeg export (`crates/server/src/routes/exports.rs:65-74`, `crates/media-engine/src/export.rs:118-134, 298-305`, `crates/server/src/routes/projects.rs:124-145`)**:
  - Resolve every referenced `MediaAsset.path` through `state.resolve_readable_path_async` before calling `execute_export_with_progress` (fixes **100% broken S3 exports** and blocks FFmpeg SSRF / `allow_external_media_reads=false` bypasses).
  - Validate `TimelineCommand::AddMedia` paths at op time so arbitrary external paths or `http://` URLs cannot be injected when `allow_external_media_reads == false`.
- [x] **Sandbox Chromium & Browser GFX Execution + Fix Linter Parity (`crates/media-engine/src/gfx_render.rs:71-210`, `packages/gfx-runtime/src/compile.ts:48-123`, `packages/gfx-runtime/src/tokens.ts:11-26`, `crates/agent-runtime/src/lint.rs:10-55`, `packages/gfx-runtime/src/lint.ts:7-37`, `crates/server/src/routes/projects.rs:124-145`)**:
  - Inject strict CSP `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src data:; font-src data:;">` in both `gfx_render.rs` and `compile.ts`.
  - Isolate Headless Chromium networking (`--host-resolver-rules="MAP * ~NOTFOUND"`).
  - Run `lint_gfx_html` inside `apply_op` on `PlaceGfx` / `PlaceClip(ClipKind::Gfx)`.
  - Overhaul `lint.rs` and `lint.ts` to share identical rules using tag-scoped regex/token matching: block `Date.now\s*\(`, `performance.now\s*\(`, `Math.random\s*\(`, `setInterval\s*\(`, `setTimeout\s*\(`, `requestAnimationFrame\s*\(`, `fetch\s*\(`, `XMLHttpRequest`, `import\s*\(`, `<script[^>]*\bsrc\s*=`, external `<iframe`/`<img`/`<link` URLs; **remove false positives** that break inline `<style>@keyframes...</style>` inside `<body>` or `<img src="data:...">` paired with inline `<script>`.
  - Strip `/*` / `*/` in `sanitizeCssValue` (`tokens.ts`), fix single-pass `</style` replacement in `compile.ts`, and escape `clip.label` in `StudioWorkspace.tsx:508` (`handleSaveClipAsAsset`).
- [x] **Close CSRF, DNS Rebinding, CSWSH, and SVG Stored XSS (`crates/server/src/routes/mod.rs:180-344`, `crates/server/src/routes/ws.rs:15-48`, `crates/server/src/routes/media.rs:393-455`, `crates/server/src/routes/brands.rs:358-427`, `crates/server/src/state.rs:229-284`)**:
  - Require `Content-Type: application/json` and validate `Origin` (and `Host`) on `/mcp` per MCP Streamable HTTP 2025-03-26 security guidance.
  - Validate `Origin` on `/api/v1/ws` upgrades.
  - Add `X-Content-Type-Options: nosniff` and `Content-Security-Policy: default-src 'none'; sandbox` on all streamed media/brand responses (blocks inline `.svg` XSS).
  - Fix WebSocket handler (`ws.rs`): use `tokio::select!` polling `socket.recv()` (handling Close/Ping/Pong cleanly so disconnected sockets never leak zombie tasks), explicitly handle `RecvError::Lagged` (log + resync/close cleanly), and send periodic Ping heartbeats.
  - Harden session auth & CORS: only include localhost dev origins in CORS when loopback or when `SOLARIS_CORS_ORIGINS` is unset; issue opaque revocable session IDs (with `Max-Age=1800` and `Secure` when behind HTTPS/non-loopback) instead of storing raw `SOLARIS_AUTH_TOKEN` in cookies; bound `media_tickets` map size; replace custom `constant_time_eq` with `ring::constant_time` (HMAC-equalized so length never leaks).
  - Fix `SOLARIS_HOST=localhost` and IPv6 (`::1`, `::`) server bind (`TcpListener::bind((host.as_str(), port))`) and add graceful `SIGTERM`/`SIGINT` shutdown (`with_graceful_shutdown`).

---

## Track 2 — Dual-Path Media Engine, FFmpeg Export & Local Intelligence

### 2.1 Critical FFmpeg & Chromium Overlay Export Fixes
- [x] **Fix Chromium `file:///` URI for relative workspaces (`crates/media-engine/src/gfx_render.rs:176-185`)**: Canonicalize `html_path` to an absolute path and percent-encode segments before building `file:///...` (fixes default `.solaris-data` workspace breaking 100% of Chromium overlays).
- [x] **Fix vanishing Chromium PNG overlays (`crates/media-engine/src/export.rs:257-267`)**: Use `eof_action=repeat` (or `-loop 1` + `shortest=1`) on single-image overlays so overlays with `start_sec >= 0` stay visible across their entire `[start_sec, end_sec]` window.
- [x] **Fix broken alpha exports (`MovProRes4444` & `WebmAlpha`) and `scale < 1.0` padding (`crates/media-engine/src/export.rs:78-90, 152-237, 500-509`)**:
  - Add `:format=rgb` / `:format=auto` on alpha-aware overlays so `vf_overlay` does not strip alpha to `yuv420`.
  - Preserve alpha across color grading (`alphaextract` + `alphamerge` around `eq=`, or apply grade only to opaque visual layers before compositing onto transparent canvas).
  - Convert to `format=rgba` *before* `pad=...:color=black@0.0` (or position scaled clips via `overlay=(W-w)/2+ox:(H-h)/2+oy:format=auto`) so PiP/scaled clips don't paint solid black over `V1`.
  - Add quality-tiered `-crf` / `-b:v` flags to `WebmAlpha` (currently defaults to 200 kbps!).
- [x] **Upgrade Dual-Path GFX & Caption Export + Bundle GSAP (`crates/media-engine/src/gfx_render.rs:57-215`, `packages/gfx-runtime/src/compile.ts`, `crates/server/src/routes/exports.rs`)**:
  - Fix overlay Z-order: composite `G1 (GFX)` *under* `T1 (TEXT)` captions (currently inverted).
  - Resolve `gfx.asset_id` against workspace `SavedCodeAsset`s whenever `gfx.inline_html` is `None`.
  - Bundle **GSAP** locally into `packages/gfx-runtime` and inject it into `compile.ts` & `gfx_render.rs` (zero CDN/network dependency) so LLM-authored GSAP timelines actually animate instead of throwing `ReferenceError: gsap is not defined`.
  - Inject bundled Sora `@font-face` and any uploaded Brand Playbook `.woff2`/`.ttf` custom fonts (as data URIs or safe local font rules) into both Studio GFX iframes and Chromium export overlays.
  - Render authentic `ClipKind::Caption` styling in both preview and Chromium export: respect `caption.words` active-word highlighting, all 4 `CaptionPreset`s (`NeonGlow`, `MonoCaps`, `Karaoke`, `Minimal`), `font_family`, `highlight_color`, `clip.transform.x/y`, and 80% 9:16 safe zones (clear of bottom 22%).
  - Support true **multi-frame RGBA sequence capture** (only for time ranges containing animated `G1`/`T1` clips, stepping `window.__solarisOnSeek(t, duration)` at project FPS or reusing static PNG when no animations/word-changes exist) so exported videos preserve real kinetic captions & GSAP motion graphics.
- [x] **Fix FFmpeg Audio, Filtergraph, Deadlocks & Scaling (`crates/media-engine/src/export.rs`, `crates/media-engine/src/analyze.rs`)**:
  - Pad sidechain key `[vkey]` with `apad=whole_dur={duration:.3}` before `sidechaincompress` so A2 music doesn't cut off the instant voice ends (`export.rs:342-347`).
  - Drain `child.stderr` concurrently on a background thread in both `export.rs:578-595` and `analyze.rs:226-271` to eliminate OS pipe deadlocks.
  - Fix `escape_ffmpeg_filter_path` (`export.rs:391-403`, `transcribe.rs:137-142`) for single quotes `'` inside `'...'` FFmpeg filter expressions (and add apostrophe-path unit tests).
  - Honor `FX` adjustment-layer `ClipKind::Grade` clips (composited/applied over their time range), `track.solo` across visual/audio/overlay tracks, `clip.transform.rotation_deg`, normalized `clip.transform.x/y` (canvas-relative, matching preview/QA), and `clip.transform.smart_reframe` crop coordinates.
  - Center Ken Burns `zoompan` (`x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)'`), allow full-clip `fade_out_sec == duration_sec`, validate clip stream kinds (`MediaKind`) before mapping `[idx:v]`/`[idx:a]`, clamp `req.target_lufs` to `[-70.0, -5.0]` and finite, use `-ss` input seeking / avoid O(N) duplicate full-file decoders on split clips, and wire detected `h264_nvenc` (plus automatic fallback to `libx264` if hardware encode fails).

### 2.2 Local Transcription, Probing, Media Intelligence & Storage Fixes
- [x] **Fix FFmpeg 8 `af_whisper` NDJSON & Whisper Word Parsing (`crates/media-engine/src/transcribe.rs:11-106, 213-232`)**:
  - Parse newline-delimited JSONL (`{"start":0,"end":2500,"text":"..."}`, converting ms → sec) emitted by FFmpeg 8 `af_whisper` `format=json` alongside single-object OpenAI/whisper.cpp JSON.
  - Preserve short zero-duration words (`start == end`, assigning minimal `0.05s` duration instead of dropping `"a"`, `"I"`, `"to"`), make raw word `start`/`end` optional so a single `null` timestamp doesn't fail the whole transcript, and return an error (not `Ok(vec![])`) when text-only JSON lacks timestamps.
- [x] **Fix Proxy, Probe, Analyze & Frame Capture Bugs (`crates/media-engine/src/analyze.rs`, `probe.rs`, `frames.rs`, `lut.rs`, `ffmpeg.rs`)**:
  - Add `-pix_fmt yuv420p` and even height `scale=-2:'trunc(min({max_height},ih)/2)*2'` to `generate_proxy_with_encoders` (fixes 10-bit Log browser playback & odd-height crashes).
  - In `probe.rs`: add `audio_stream.duration` fallback, read video display rotation (`side_data_list[].rotation` / `tags.rotate`), reject non-finite durations, and replace `-v quiet` with `-v error` so failure stderr is populated.
  - In `analyze.rs`: filter `.is_finite()` on `ebur128` LUFS (guard `-inf`), clamp negative `silencedetect` timestamps to `>= 0.0`, and pass duration to `detect_silences` so EOF silences aren't dropped.
  - In `frames.rs`: verify `output_png.is_file()` and non-empty bytes after FFmpeg exits `0`; guard `max_width == Some(0)`.
  - In `lut.rs`: strip UTF-8 BOM (`\u{FEFF}`), handle `#` inside quoted `TITLE`, support `LUT_3D_INPUT_RANGE`, reject `DOMAIN_MIN >= DOMAIN_MAX` and duplicate/misplaced `LUT_3D_SIZE`.
  - In `ffmpeg.rs:105`: use `path.is_file()` instead of `path.exists()`.
- [x] **Implement Missing Local Media Intelligence Features (`crates/media-engine/src/analyze.rs`, `crates/core-ir/src/model.rs`)**:
  - Populate `MediaIntelligenceManifest.beats` via lightweight Rust onset/energy peak detection on the already-decoded PCM audio stream (zero extra FFmpeg pass) so beat-synced cuts work in Faceless Explainers & Reels.
  - Add FFmpeg `scdet` scene-cut detection (`scene_cuts: Vec<f64>`) and default smart `9:16` subject/reframe crop estimation so `SmartReframeBox` works end-to-end.
- [x] **Harden `LocalFsStorage`, `S3Storage` & Export Job Files (`crates/media-engine/src/storage.rs`, `s3_storage.rs`, `crates/server/src/routes/exports.rs`, `media.rs`)**:
  - Fix `Path::parent() == Some("")` `ENOENT` across all 5 `create_dir_all` call sites (`engine.rs`, `analyze.rs`, `transcribe.rs`, `frames.rs`, `export.rs`).
  - `LocalFsStorage`: check symlink confinement *before* `create_dir_all`, handle `Component::Prefix`, clean up `.tmp` on rename failure, fail cleanly if `canonicalize(root)` fails.
  - `S3Storage`: add symlink confinement in `get_object_to_cache_sync`, remove local cache file if `put_bytes_sync` fails, honor `force_path_style = true` even when `endpoint` is `None`, percent-encode bucket/base_path, trim `session_token` consistently, reuse a shared `reqwest::Client` + worker runtime instead of spawning a new OS thread/runtime/TLS store per call, avoid `key_prefix` cache aliasing, and surface S3 errors instead of swallowing them.
  - Always include `&job_id[..8]` in export output file paths and enforce an ASCII allowlist (`[a-zA-Z0-9._-]`, max 128 chars) in `sanitize_file_name` (eliminates concurrent file corruption and `Content-Disposition` header injection); keep `export_jobs` updated with live progress and never evict running export jobs or swarm runs at the 64-item cap.

---

## Track 3 — Core IR Integrity & Server State Robustness

- [x] **Preserve `media_pool[*].intelligence` across `undo()`/`redo()` & background analysis (`crates/core-ir/src/engine.rs:165-293`, `crates/server/src/routes/media.rs:215-294`)**:
  - On `undo()`/`redo()`, preserve current `intelligence` for surviving `media_pool` IDs so undoing a clip edit never erases transcripts, waveforms, silences, or `proxy_path`.
  - In Stage 1 & Stage 2 background media analysis (`routes/media.rs`), re-read current `asset.intelligence` inside the write lock and merge fields rather than committing a stale snapshot (prevents proxy completion from wiping concurrent transcripts). In `stream_media`, fall back to `asset.path` if `proxy_path` cannot be resolved.
- [x] **Enforce User Track Locks, ClipKind Safety & Float Finiteness (`crates/core-ir/src/engine.rs:141-153, 297-364, 437, 517-610`, `commands.rs:156`)**:
  - Only allow `AgentRole::User` to unlock or mutate flags on a `track.locked == true` track.
  - Guard `SetColorGrade` and `UpdateCaptionClip` so they require matching `ClipKind::Grade` / `ClipKind::Caption` payloads and reject partial `(Some, None)` tuples; check `fx` track lock on project-level `SetColorGrade`.
  - Validate `.is_finite()` and domain bounds (`scale > 0`, `opacity ∈ [0, 1]`, `fade >= 0`, `smart_reframe.width/height > 0`) recursively across all incoming IR structs (`PlaceClip`, `UpdateClipTransform`, `SetColorGrade`, `UpdateCaptionClip`, `AddMedia`, `UpdateMediaIntelligence`) so `NaN`/`Inf` can never brick `project.json`.
  - Re-sort `track.clips` after `SplitClip` and call `sync_all()` + `.tmp` cleanup in `save_project_atomic`.
- [x] **Fix Server Concurrency, Locking & Route Hygiene (`crates/server/src/*`)**:
  - Emit WS `OpCommitted`/`ProjectReset` inside the `projects.write()` lock (preserving strict monotonic revision order on the wire).
  - Move blocking JSON/file/LUT disk I/O off Tokio async locks (`spawn_blocking`), keep `upsert_brand` & `save_asset` memory+disk updates atomic, validate `save_asset` with `validate_single_asset`, sync `brand.default_lut` on `update_brand_media`, and fix `delete_brand_media` under S3 + scope path deletion to that brand's directory.
  - Fix `cancel_swarm_run` to verify `run_id` belongs to `project_id`; use unique UUID temp filenames for OpenRouter STT `{mid}_{uuid}.mp3` and validate `media_id` with `validate_safe_id`.
  - Scope 2 GiB `DefaultBodyLimit` strictly to multipart uploads (not JSON bodies), make `multipart/form-data` detection case-insensitive, clean up partial files on interrupted uploads, ignore syntactically invalid `Range` headers per RFC 9110 (`200 OK` instead of `416`), check `metadata.is_file()` before streaming, fix `serve_static_ui` non-GET/prefix handling + MIME types, fix HTTP status mapping (`404` for missing track/project/file instead of `409`/`403`/`400`), log corrupted JSON files on startup, and remove dead `AppState::persist_project`.

---

## Track 4 — OpenRouter BYOK, Model Advisor, True Multimodal Swarm & QA

### 4.1 BYOK OpenRouter API Key, OS Keychain & Model Advisor UI (Core Vision Gaps)
- [x] **In-App OpenRouter API Key Management + Desktop OS Keychain (`apps/desktop/src-tauri/src/lib.rs`, `crates/server/src/state.rs`, `crates/server/src/routes/agent.rs`, `packages/engine-client/src/client.ts`, `apps/studio-web/src/components/*`)**:
  - Support storing/loading/clearing `OPENROUTER_API_KEY` via native OS Keychain (`keyring` crate) in Tauri desktop mode, forwarded securely to the embedded server at startup and updatable at runtime via an authenticated server endpoint (`POST/DELETE /api/v1/agent/credentials`, never returned in plain text or stored in `localStorage`).
  - Add a clean **AI / OpenRouter Settings** modal in `PreEditorChat` and `StudioWorkspace` where users enter their OpenRouter API key (or choose External MCP Agent mode with 1-click copy & `solaris-mcp install-skills` instructions).
- [x] **Model Advisor & Model Selector UI (`apps/studio-web/src/components/chat/PreEditorChat.tsx`, `StudioWorkspace.tsx`, `crates/agent-runtime/src/advisor.rs`)**:
  - Wire `client.getRecommendedModels()` into `PreEditorChat` and `StudioWorkspace` so users see recommended models with a **Best Match (Video + Audio + Image + Text + Tools)** omni-modal badge & clear explanation ("Sees your raw footage, hears pacing, and inspects QA frames natively"), plus **Smart Fallback (Image + Text + Tools)** guidance for Claude/GPT vision models, and can pick a model override persisted for swarm runs.
  - Fix `ModelAdvisor` (`advisor.rs:127-243`): pass `output_modalities=Some("all")` to `list_models` so `TranscriptionStt` discovers real live STT models; filter `outputs.contains("text")` for Director/Worker; penalize/exclude rate-limited `:free` endpoints from winning top spot; add negative/stale-while-error caching (prevents 180–270s serial hangs); handle JSON `null` values in `OpenRouterModel` (`openrouter.rs:265-298`).

### 4.2 True Multimodal Context, Brand Playbook Injection & OpenRouter Reliability
- [x] **Inject Full Brand Playbook & Attached Script Documents into Director & Swarm (`crates/agent-runtime/src/swarm/llm.rs:52-90, 196-255`)**:
  - Pass `brand.brand_md`, `brand.tokens`, `brand.pacing_rules`, `brand.default_lut`, and `brand.reference_media` notes to both `build_openrouter_director_plan` AND `run_llm_worker_loop`.
  - Read attached `MediaKind::Document` files (`.md`, `.txt`, and `.pdf` text extraction) and pass bounded script/brief excerpts into the Director prompt context.
- [x] **Native Multimodal Video/Audio Packaging for Omni-Modal Models (`crates/agent-runtime/src/swarm/llm.rs`, `crates/media-engine/src/analyze.rs`)**:
  - When the chosen Director/QA model reports native `video` + `audio` `input_modalities`, generate/attach a compact low-bitrate 480p proxy (or base64 `video_url` / `input_audio` within size budget per OpenRouter provider rules); automatically fall back to compressed keyframe contact sheets + full word-level transcript + silence/beat JSON for image+text models.
- [x] **Harden OpenRouter Client & STT (`crates/agent-runtime/src/openrouter.rs`, `error.rs`, `stt.rs`)**:
  - Detect HTTP 200 JSON `{"error": ...}` bodies in both `chat_stream` and `chat_complete` instead of returning empty `Ok`.
  - Fix `SseAccumulator` tool-call assembly: use a `BTreeMap<usize, ToolCall>` (capped index, no phantom empty slots), handle missing `index` by keyed `id`, support multi-line `data:` SSE events per spec, reject truncated JSON args on EOF instead of silently passing `{}` to `execute_tool`.
  - Replace total `.timeout(90s)` with `connect_timeout` + `read_timeout` for streaming; add `500`, `504`, and `520..=524` to `is_retryable`; add retries, fallback models, `truncate_msg`, `config.timeout_secs`, and `.mp4` MIME support in `stt.rs`.

### 4.3 Swarm Orchestration, Tool Accuracy & Stage-3 QA Loop
- [x] **Fix Swarm Transcription, Vision Feedback, Track Locks & Fallbacks (`crates/agent-runtime/src/swarm/mod.rs`, `llm.rs`, `deterministic.rs`, `tools/schemas.rs`)**:
  - Auto-transcribe untranscribed speech assets (`has_audio && transcript.is_empty()`) at Stage 1 before Director/Razor/Caps run (and include `media.transcribe` + `preview.capture_frame` + `media.get_manifest` in role tool sets to match `skills/*/SKILL.md`).
  - Return `ToolOutput::image_png_base64` as multimodal image content in `run_llm_worker_loop` so workers calling `preview.capture_frame` actually see the frame.
  - Wrap track locks in RAII drop-safe cleanup guards (preventing 300s lock leaks on panic/abort); propagate `CancellationToken` into advisor, keyframe extraction, deterministic stages, multi-tool turns, and QA inner loops; enforce track-scope restrictions during Stage-3 specialist fan-out.
  - Detect 0-op / all-failed LLM worker turns and trigger local fallback (properly setting `status.used_local_fallback = true`); fail the run cleanly (not `Completed`) if all specialists fail with `allow_local_fallback: false`; fix `strip_json_fence` to extract JSON from prose-wrapped responses.
  - Fix `deterministic.rs`: place `voice` audio onto `A1` (never `A2` ducked music), clamp `A2` music duration to video/voice timeline span, handle images-only / b-roll-only / `<2.5s` videos / empty media / unsorted silences / template re-runs without overlaps or black lead/tail gaps, and avoid committing no-op `SetColorGrade` ops.
- [x] **Fix Tool & QA Bugs (`crates/agent-runtime/src/tools/exec.rs`, `crates/agent-runtime/src/swarm/qa/*`)**:
  - `assets.search`: search `a.id` alongside `a.name` and `a.tags` (matches `SKILL.md` references like `chapter-title-solar`).
  - `assets.save`: run `solaris_core_ir::validate_single_asset`; `gfx.place`: reject unknown `asset_id` / empty source and deep-merge partial `props` over `default_props`.
  - `project.create`: preserve template's native `aspect_preset` (`16:9` for `faceless-explainer`) when `aspect_preset` is omitted; return validation errors on unknown enum values instead of silently defaulting.
  - `timeline.place_clip`: validate `media_id` exists in `media_pool`, `track_id` matches `ClipKind`, and `scale > 0`.
  - `captions.generate_from_transcript`: map against both `V1` and `A1` tracks (fixes Faceless Explainer sync), replace existing auto-captions to avoid stacking duplicates, split bursts across silence gaps (`> 0.6s`), prevent split-boundary word duplication, prevent `.max(0.25)` overlap into next burst, and set safe-zone `transform.y` (`0.20`) instead of `0.0`.
  - `audio.normalize_lufs` & `qa/checks.rs`: skip muted tracks and clips with `gain_db <= -90.0`; in QA, skip re-applying fixes when gain is already clamped at `+12 dB` (stops infinite QA fix loops); reject negative fades in `audio.set_mix`.
  - `qa/checks.rs` & `qa/vision.rs`: check GFX safe-zone bounds too, avoid colliding a shifted GFX with a second overlapping caption, fix pacing check for overlapping clips / `V2` B-roll coverage, fail `project.run_qa` cleanly on missing project IDs, capture graded/composited frames (not raw ungraded source) for Vision QA, and surface VLM errors as warnings instead of swallowing them.

---

## Track 5 — Pre-Editor Chat, Live NLE Studio Preview, Audio & Hybrid Swarm Cursors

### 5.1 Pre-Editor Chat & Intake Experience (Core Vision Gaps)
- [x] **Background Indexing on File Drop + Director Plan Card (`apps/studio-web/src/components/chat/PreEditorChat.tsx`, `apps/studio-web/src/App.tsx`)**:
  - Create draft project & begin uploading/analyzing media as soon as files drop in `PreEditorChat`, rendering live progress pills (`Indexed N clips · Transcribed M words · Found K silences`).
  - Add an optional lightweight **Pre-Editor Shotlist / Plan** step (or direct launch with a server/client readiness barrier waiting for Stage-1 media analysis + WS connection before starting the Swarm — fixes C8 race).
  - Fix `PreEditorChat` layout (`min-h-dvh` / scrollable container on short viewports), empty-prompt project naming (`"Untitled Solaris Edit"` shouldn't auto-run Swarm as a fake user prompt), and template prompt slug truncation.

### 5.2 Real Audio Playback, Smooth WebGL2 Stage, Live Scopes & Captions/GFX Preview
- [x] **Implement Real Web Audio Timeline Playback & Live Metering (`apps/studio-web/src/components/studio/WebGLProgramStage.tsx`, `StudioWorkspace.tsx`, `BroadcastScopes.tsx`)**:
  - Add a playhead-synchronized Web Audio API engine routing active `V1` (when unmuted & no explicit voice override), `A1 (VOICE)`, and `A2 (MUSIC)` tracks with clip `gain_db`, fades, track mute/solo, and live ducking attenuation.
  - Drive `BroadcastScopes` VOICE/MUSIC bars and integrated loudness from real `AnalyserNode` levels + active timeline clips (not static config numbers).
- [x] **Fix WebGL2 Color Pipeline, Video Sync, Transforms & Scope Performance (`WebGLProgramStage.tsx`, `StudioWorkspace.tsx`, `BroadcastScopes.tsx`)**:
  - Fix 3D LUT half-texel sampling (`lutCoord = (lutCoord * (u_lutSize - 1.0) + 0.5) / u_lutSize`), trigger immediate redraw on LUT load, and resolve LUT/grade from `FX` adjustment clips → `project.active_grade.lut_path` → `activeBrand.default_lut`.
  - Fix video seek dead-zone (`0.12s` paused scrub), clip-switch frozen frames (`readyState`), infinite seek loop when `targetSec > vid.duration`, and gap/muted-track `selectedMedia` leak.
  - Render full `Transform2D` (`x`, `y`, `scale`, `rotation_deg`, `opacity`, `smart_reframe`, `ken_burns`) and composite `V2` over `V1` cleanly; handle `webglcontextlost`/`webglcontextrestored`.
  - Remove duplicate 60fps `drawCurrentFrame` texture uploads and remove `playheadSec` from `BroadcastScopes` `drawWaveform`/`drawVectorscope` deps so scopes render at 15fps without thrashing the main thread; fix stale `"0–100 IRE"` → `"LIVE"` badge.
- [x] **Authentic Captions Preview, Brand Fonts & Saved Asset Thumbnails (`WebGLProgramStage.tsx`, `StudioWorkspace.tsx`, `packages/gfx-runtime/*`)**:
  - Render word-synced `caption.words` with active-word highlighting, preset styles (`NeonGlow`/`MonoCaps`/`Karaoke`/`Minimal`), `font_family`, `highlight_color`, and `transform.x/y` inside the 80% safe zone (above bottom 22%).
  - Inject `@fontsource-variable/sora` + Brand Playbook uploaded `.woff2`/`.ttf` `@font-face` into GFX `srcDoc` iframes; include `clip.start_sec` in `gfxKeySignature`; handle `solaris:gfx-ready` handshake; support `schemaDef.default` and color/object prop defaults cleanly in `packages/gfx-runtime/src/schema.ts`.
  - Render miniature live/cached visual previews for Saved Code Assets in the Effects · GFX bin (`back and forth.md:173, 350`).

### 5.3 Essential NLE Controls, Hybrid Cursors & UI Polish
- [x] **Complete Core Manual NLE Editing Controls (`StudioWorkspace.tsx`, `TimelinePanel.tsx`, `InspectorPanel.tsx`)**:
  - Add **"Add to Timeline"** (button, double-click, and drag-and-drop) from Media Pool onto `V1`/`V2`/`A1`/`A2`.
  - Add clip drag/move, edge trim/resize handles, timeline zoom controls, and track `solo` toggle in `TimelinePanel`.
  - Add Inspector controls for `Transform2D` (Position X/Y, Scale, Rotation, Opacity), `AudioSettings` (Gain dB, Ducking toggle/target, Fade In/Out), and Caption text/words editing; wire Source Monitor `9:16` reframe box overlay proportionally to project aspect + working IN/OUT trim markers.
- [x] **Fix Hybrid Real + Choreographed Swarm Cursors (`SwarmCursorLayer.tsx`, `StudioWorkspace.tsx`)**:
  - Enable subtle choreographed scanning micro-motions during **active real Swarm runs** (LLM thinking gaps), not only in `demoMode`; expire/fade idle cursors cleanly after inactivity.
  - Add `data-cursor-anchor="timeline"` on the timeline container and wrap anchor lookup with `CSS.escape(anchor)`.
- [x] **Fix Studio UI State Races, Shortcuts & Edge Cases (`App.tsx`, `StudioWorkspace.tsx`, `ExportModal.tsx`, `SwarmActivityPanel.tsx`, `ColorWheelControl.tsx`, `InspectorPanel.tsx`, `TimelinePanel.tsx`, `BrandPlaybookDrawer.tsx`)**:
  - Fix HTTP `202` vs WS race in `ExportModal` and `SwarmActivityPanel` (never regress terminal/newer status to `"queued"`, disable Render while active, poll status on WS reconnect).
  - Add monotonic `revision` guards on `setActiveProject` in `App.tsx`, upsert (not duplicate) updated assets, call `client.dispose()` on unmount, and notify React when `media_ticket` refreshes (fixes Tauri 30-minute video/export auth expiry).
  - Fix `ColorWheelControl` `Backspace` propagation (`e.stopPropagation()`), ignore right-click drags, avoid 3-op double-click reset & arrow-key op spam; disable global keyboard shortcuts when modals/drawers are open or `<button>`/`<a>` is focused; fix `handleUndo`/`handleRedo` effect deps.
  - Fix `InspectorPanel` mid-keystroke `localProps` resets on unrelated WS ops, clear debounce timer on unmount, fix LUT slider keyboard commit & Tab `onKeyUp` spam, fix empty-project Swarm Role bars (`?? 0`), and surface error toasts/banners instead of unhandled rejections on locked-track edits.
  - Fix `TimelinePanel` exclusive end boundary (`playheadSec < start + dur`), clamp Play start to `totalDurationSec`, slice waveform peaks proportionally across `source_in_sec..source_in_sec+dur`, and compute real hours in `formatTimecode`.
  - Fix `BrandPlaybookDrawer` draft reset on reopen/cancel, controlled reference note inputs, and reset file `<input value="">` after upload; guard `SwarmActivityPanel` Enter key while running, separate `errorMsg` vs `qaError`, handle rejected `clipboard.writeText`, clamp `ExportModal` LUFS cleanly without mid-keystroke reset, catch synchronous `new WebSocket` throws in `client.ts`, and add `motion-reduce:transition-none` on remaining Tailwind transitions.

---

## Track 6 — Tauri Desktop Polish, MCP Compliance, Cleanup & Docs Sync

- [x] **Harden Tauri v2 Desktop Wrapper (`apps/desktop/src-tauri/*`)**:
  - Eliminate startup race: set `"create": false` on the main window in `tauri.conf.json`, spawn embedded `solaris-server` and call `app.manage(info)` *before* creating the window at the end of `.setup()`.
  - Store server shutdown sender and trigger graceful shutdown on `RunEvent::Exit`.
  - Add native desktop export Save/Download handling so macOS WKWebView downloads work reliably instead of ignoring cross-origin `<a download>`.
  - Update Tauri CSP (`tauri.conf.json:24`) to allow Brand Playbook custom fonts from `http://127.0.0.1:*` and tighten `script-src`; generate real `.icns`/`.ico` icon assets alongside `icon.png`; remove unused direct `tokio` dependency from `apps/desktop/src-tauri/Cargo.toml`.
- [x] **MCP Protocol & CLI Compliance (`crates/mcp-server/*`)**:
  - Handle JSON-RPC `"id": null` explicitly (return `-32600` instead of dropping as notification); fix `-32600` (invalid request), `-32602` (unknown tool), and `-32002` (resource not found) error codes; gate `structuredContent` on protocol version `>= 2025-06-18`.
  - Track real MCP sessions in `http.rs` (`DELETE /mcp` invalidates session); make `run_stdio_http_bridge` (`stdio.rs`) concurrent with HTTP timeouts and single-line JSON output verification; reject unknown flags/targets in `solaris-mcp install-skills` and support Windows `USERPROFILE`; delete dead `crates/mcp-server/src/http_backend.rs`.
- [x] **Docs Sync, Dead Code Cleanup & E2E Tests**:
  - Sync docs with reality: update "32 tools" → **33 tools** and add `media.transcribe` in `docs/MCP_AND_SKILLS.md`, document `SwarmProgress` and `PlayheadSeek` in `docs/TIMELINE_IR.md`, fix duplicate `### 2.` headers in `README.md`, and remove unused dead code (`ProjectSummaryItem`, `OpenRouterClient::chat_complete`, `AppState::persist_project`).
  - Add targeted regression tests covering every Track 1–5 critical fix (alpha export, overlay `eof_action=repeat`, canonical `file:///` URI, S3 export resolution, `af_whisper` NDJSON, `undo()` intelligence preservation, `lint.rs`/`lint.ts` parity, `ModelAdvisor` STT discovery, `SetColorGrade` safety, and WebSocket lag/close).

---

## Production-Readiness Audit Hardening (Pass 2)

- [x] **Persistent CDP Chromium & Dual-Path Overlay Engine (`crates/media-engine`)**: Replaced per-frame cold Chromium launches with persistent CDP WebSocket page reuse, added `render_single_frame_overlays_via_chromium` + `capture_composited_frame_png` with `CompositeFrameOptions`, fixed unquoted 2-level FFmpeg path escaping, and added universal `-filter_complex` / `-/filter_complex` / `-filter_complex_script` compatibility across FFmpeg 5–9.
- [x] **Core IR & Agent Runtime Hardening (`crates/core-ir`, `crates/agent-runtime`, `crates/mcp-server`, `skills/`)**: Added `TimelineCommand::RemoveMedia`, `ProjectEngine::update_metadata`, strict HTML/float/ID validation, multimodal image/audio helpers on `SolarisBackend`, RAII async track lock release (`spawn_release_track_lock`), prompt-caching & SSE hardening, and stateless vs stateful MCP protocol isolation.
- [x] **Server Security, Concurrency & Sandbox Endpoints (`crates/server`)**:
  - Graceful WebSocket shutdown via `AppStateInner.shutdown: CancellationToken` wired into `main.rs`, `EmbeddedServerHandle`, and `routes/ws.rs`.
  - DNS-rebinding `Host` checks across `auth_guard` and `stream_or_ws_auth_guard`, cross-site `Origin` enforcement on mutating HTTP methods, `415 Unsupported Media Type` JSON guard on media/brand imports, logout revocation of session-linked `media_ticket`s with HTTPS `Secure` flag, and static UI `X-Content-Type-Options`/`Cache-Control` headers.
  - Shared `AppState::validate_timeline_command` across REST and `SolarisBackend::apply_op` (validating `AddMedia`, `UpdateMediaIntelligence`, `SetColorGrade`, `PlaceClip`, and `PlaceGfx` paths/HTML).
  - New routes: `PATCH /api/v1/projects/{id}`, `DELETE /api/v1/projects/{id}/media/{media_id}`, and `POST/GET /api/v1/gfx/sandbox` (128-entry LRU, 512 KiB max, strict sandboxed CSP).
  - Eliminated global `brands`/`assets` `RwLock` held across disk `fsync` via `brand_persist_lock`/`asset_persist_lock`, gated exports with `SOLARIS_MAX_CONCURRENT_EXPORTS` semaphore, failed exports cleanly on missing Chromium/overlay render errors, blocked overlapping Swarm runs per project with `409 Conflict`, and added `/FlateDecode` zlib PDF text extraction + RAII temp file cleanup in `backend.rs`.
- [x] **Frontend & GFX Sandbox Hardening (`packages/gfx-runtime`, `packages/engine-client`, `apps/studio-web`)**: Migrated Studio GFX previews to server-served `/api/v1/gfx/sandbox/{id}` iframes with strict CSP, stabilized WebGL2 refs/lifecycle, added project rename & media delete flows, and ensured zero Vite >500 kB chunk warnings.
- [x] **Tauri Desktop & Docker/Cloud Polish (`apps/desktop/src-tauri`, `docker-compose.yml`, `.env.example`)**: Updated Tauri CSP `frame-src` for `http://127.0.0.1:*`, made `save_export_file` async with `spawn_blocking` + atomic `OpenOptions::create_new` + UUID collision fallback, bounded `RunEvent::Exit` server shutdown to 3s with abort fallback, pinned MinIO release tag, and documented all new container/runtime env vars.

---

## Track 7 — Solar Horizon UI Polish & Agent Skills Overhaul

### 7.1 Pre-Editor & NLE Studio UI Adjustments
- [x] **Reusable Styled Select (`apps/studio-web/src/components/ui/SolarSelect.tsx`)**: Extract custom-chevron `appearance-none` select primitive to eliminate mismatched native OS dropdown arrows across Pre-Editor, AI Settings, Brand Playbook, and Export modals.
- [x] **Pre-Editor Landing & Composer (`apps/studio-web/src/components/chat/PreEditorChat.tsx`)**: Unify hero + composer into a single vertically balanced stage (eliminating the empty vertical void), fix bottom toolbar wrapping so the submit button stays anchored cleanly on the right in one row, consolidate fragmented AI/Model Advisor chrome into a sleek single-line selector pill, move starter templates beneath the composer as quick-start chips, align headline to `DESIGN.md` fluid scale, remove em-dashes and noisy engine version jargon from the top bar, and unify header pill heights + divider.
- [x] **Studio NLE Layout, Program Monitor & Timeline (`StudioWorkspace.tsx`, `WebGLProgramStage.tsx`, `TimelinePanel.tsx`, `InspectorPanel.tsx`, `ColorWheelControl.tsx`, `SwarmActivityPanel.tsx`)**: Give the Program Monitor Media-First column dominance with non-overflowing responsive grid tracks, fix 16:9/9:16/1:1 aspect-ratio distortion via `ResizeObserver` fit math in `WebGLProgramStage`, fix 160px horizontal overflow at 1.0× zoom in `TimelinePanel` + add clickable timecode ruler & playhead cap, prevent Inspector color-wheel overflow at 1080–1280px, center GFX thumbnails, fix GFX/Swarm header cramping, and lift all sub-10px typography (`text-[8px]`/`text-[9px]`) to `text-[10px]`+.
- [x] **Unified Viewer & Wider Bins (`StudioWorkspace.tsx`, `SourceMonitor.tsx`)**: Merged the separate Source and Program monitors into one viewer with `Program | Source` tabs (clicking a Media clip opens Source; play/seek/Insert Range return to Program), extracted IN/OUT trim into `SourceMonitor`, gave Director & Swarm its own full-height column, widened the Media/GFX column (320px xl) with both bins splitting height and a 2-column GFX thumbnail grid. Removed the two saved `three_ai_tools_reel_(demo)` projects from `.solaris-data`.
- [x] **Modals & Drawers (`ExportModal.tsx`, `AiSettingsModal.tsx`, `BrandPlaybookDrawer.tsx`)**: Add backdrop click-to-dismiss, align Export `grid-cols-2` labels/inputs cleanly, remove redundant double model selector in `AiSettingsModal`, and apply `SolarSelect`.

### 7.2 Solaris Built-In & External Agent Skills Overhaul (Benchmarked vs `motion-director` & `skills-for-reference`)
- [x] **Audit & Upgrade `skills/` (`solaris/skills/*`)**: Benchmark Solaris's built-in skills against `motion-director` (progressive disclosure, decision gates, numbers-only specs, banned-defaults lists, multi-round critique/QA loop, deterministic verification, token economy, exact MCP/REST/tool workflows) and upgrade/restructure Solaris's skills + `references/` so internal OpenRouter agents and external LLM agents (Codex, Antigravity, Claude Code) drive Solaris cleanly end-to-end.
- [x] **Sync Installer, Embedded Skills, Runtime & Docs**: Ensure `solaris-mcp install-skills` installs all skill files AND `references/` subdirectories cleanly for Antigravity, Codex, and Claude Code, keep `crates/agent-runtime` skill injection and `docs/MCP_AND_SKILLS.md` in sync, and validate every skill with `validate_skill.py`.
- [x] **Full Verification, Commit & Push**: Run `cargo fmt`, `cargo clippy`, `cargo test`, Tauri check, `npm run typecheck`, `npm test`, `npm run build`, and `audit_taste.py`, update `tasks/lessons.md`, then commit and push to `origin/main`.

---

## Track 8 — OpenRouter Reliability, Diagnostic Logs & Zero-Asset Programmatic Video Production

- [x] **Diagnostic Logs Pipeline (`core-ir`, `agent-runtime`, `server`, `engine-client`, `studio-web`)**: Add `LogLevel` + `LogEntry` + `EngineEvent::LogEmitted`, bounded server ring buffer + `GET/DELETE /api/v1/logs`, emit structured Swarm/OpenRouter/Tool/QA/Export logs (model, `finish_reason`, token/reasoning usage, raw parse failure context, never secrets), and add a dedicated **Logs** section + expandable Logs Drawer in Studio UI with error badges, filtering, and copy-all.
- [x] **Fix OpenRouter Malformed/Truncated Plan JSON (`openrouter.rs`, `swarm/llm.rs`, `advisor.rs`, `qa/vision.rs`)**: Capture `usage` + `reasoning_tokens` + `finish_reason` from OpenRouter SSE/JSON responses, support `response_format` when model advertises it, raise `max_tokens` (Director `4096`/`8192` retry, Worker `8192`, Vision QA `1024`), make `DirectorPlan` resilient with `#[serde(default)]`, add 1-shot automatic retry on truncated/malformed Director JSON, and reward `structured_outputs`/`response_format`/`tool_choice` in `ModelAdvisor`.
- [x] **Zero-Asset Programmatic Video Production (`swarm/`, `tools/`, `media-engine`, `server`, `studio-web`, `skills/`)**: Detect empty visual media pools and run full programmatic video production (multi-scene full-canvas responsive HTML/CSS/SVG/GSAP motion graphics on `G1` covering `0.0s..target_duration_sec` + new `captions.place_script` tool for synchronized kinetic captions on `T1` + upgraded deterministic zero-asset fallback + overlay-only `capture_preview_frame` for Vision QA + hide empty stage placeholder when GFX/captions are active + update `skills/`).
- [x] **Verification & Lessons**: Add targeted unit tests, run full verification (`cargo fmt`, `cargo clippy`, `cargo test`, Tauri check, `npm run typecheck`, `npm test`, `npm run build`), and update `tasks/lessons.md`.

---

## Track 9 — Demo Studio Overhaul, Transparent GFX & Stage Scaling

- [x] **Fix Opaque White Iframe Background & Resolution-Independent GFX/Caption Stage (`packages/gfx-runtime`, `crates/media-engine`, `apps/studio-web`)**: Add `color-scheme: dark` to sandboxed GFX documents to eliminate Chromium/WebKit opaque-white canvas mismatch, render GFX overlays in a canonical virtual stage scaled to the fitted monitor, scale captions proportionally, and add GSAP entrance/exit choreography to starter code assets.
- [x] **Real Playable Demo Media Generator & Canonical Demo Endpoint (`crates/media-engine/src/demo_media.rs`, `crates/server`, `packages/engine-client`)**: Synthesize and cache real playable 9:16 video clips (FFmpeg animated studio plates with pure-Rust PNG fallback) + real 44.1kHz stereo WAV audio stems (118 BPM musical groove + synced narration cadence), and expose `POST /api/v1/projects/demo` building a complete 16s 7-track showcase under stable ID `solaris-demo-reel` (preventing duplicate `Recent` entries).
- [x] **Studio UX, Restrained Cursors, Scopes & Timeline Span (`apps/studio-web`)**: Clamp Swarm cursors inside viewport bounds, remove `program.stage` blocking anchor, default to clean Focus mode on open (auto-showing cursors on real Swarm runs), fix `BroadcastScopes` header wrapping, eliminate empty trailing timeline dead space, initialize Demo playhead at `1.2s` hero frame, and clarify `Open Demo Studio` CTA.
- [x] **Verification & Cleanup**: Remove stale duplicate demo JSONs, add regression tests, run full Rust/TS/Tauri verification suite, and record lessons in `tasks/lessons.md`.

---

## Track 10 — Programmatic GFX Reliability, Honest Vision QA & Broadcast-Grade Captions

- [x] **Fix Model Routing, Specialist Truncation Recovery & GFX Cadence (`crates/agent-runtime/src/advisor.rs`, `swarm/llm.rs`, `swarm/mod.rs`, `skills/captions-and-gfx/SKILL.md`)**:
  - Exclude `:batch` OpenRouter variants from interactive Director/Specialist recommendations in `ModelAdvisor`.
  - Raise `WORKER_MAX_TOKENS` to `16384`, recover from mid-stream tool-call truncation (`StreamError`) inside `run_llm_worker_loop` by rotating fallback models and coaching 1-scene-per-turn `gfx_place` calls (skipping redundant `gfx_lint_html`), nudge `gfx-01` if `g1` coverage is incomplete, and retry a failed critical `gfx-01` pass once on the Director/fallback model before giving up.
  - Instruct `caps-01` that Director runs QA automatically so it stops wasting turns calling `project_get`.
- [x] **Make Vision QA & Pacing Checks Honest on Missing Visuals (`crates/agent-runtime/src/swarm/qa/checks.rs`, `vision.rs`, `mod.rs`, `swarm/mod.rs`)**:
  - Emit a hard `QaSeverity::Issue` (`visuals-empty`) when `V1`, `V2`, and `G1` all have 0 active clips, and `g1-coverage-low` when programmatic `G1` leaves large uncovered gaps.
  - Skip shifting full-canvas (`100vw`/`100vh`/`inset:0`) `G1` scenes during caption anti-collision in `check_and_fix_safe_zones`.
  - Upgrade `VisionQaResponse` to support structured `issues` (`QaSeverity::Issue`) alongside `warnings` (`QaSeverity::Warning`), pass timeline/programmatic context to the VLM, and feed `specialist_errors` into QA so failed specialists or pitch-black frames never pass at `85/100`.
- [x] **Fix Caption Font Fallback, Smart Clause Splitting & Preview/Export Parity (`crates/agent-runtime/src/tools/exec.rs`, `crates/media-engine/src/gfx_render.rs`, `apps/studio-web/src/components/studio/WebGLProgramStage.tsx`)**:
  - Resolve `"Sora"` → `"Sora Variable", "Sora", var(--font-sans)` in `WebGLProgramStage.tsx` so captions never fall back to system serif fonts.
  - Preserve segment boundaries in `captions.place_script`, break bursts cleanly on sentence/clause punctuation, and balance 4-word/orphan chunks (`2+2` instead of `3+1`).
  - Align `transform.y` vertical mapping and elevate `NeonGlow`/preset typography & contrast across Studio preview (`WebGLProgramStage.tsx`) and Chromium export (`gfx_render.rs`).
- [x] **Targeted Tests, Full Verification & Lessons**: Add unit tests for `:batch` filtering, punctuation/segment caption splitting, empty-visuals & Vision `issues` QA scoring, full-canvas anti-collision guard, and run `cargo fmt`, `cargo clippy`, `cargo test`, `npm run typecheck`, `npm test`, `npm run build`.

---

## Track 11 — Dynamic Parallel Micro-Agent Swarm, Low-Latency Reasoning & Live Timecode Cursors

- [x] **OpenRouter Low-Latency Worker Reasoning & Structured Scene Plan (`crates/agent-runtime/src/openrouter.rs`, `swarm/mod.rs`, `swarm/llm.rs`)**:
  - Add optional `ReasoningConfig` (`effort: "low"`) to `ChatRequest`, gated strictly on `advisor.model_supports_parameter(model, "reasoning")` so fast workers avoid 2,000+ token reasoning stalls without breaking non-reasoning endpoints.
  - Extend `DirectorPlan` with `art_direction` and structured `scenes: Vec<SceneSpec>` (with backward-compatible `[start-end]` `gfx_notes` parser + contiguous normalization so all scenes span `0.0s..target_duration_sec` with zero gaps).
- [x] **Parallel Scene Micro-Agent Fan-Out (`JoinSet`) & Wave 1/2 Footage Overlap (`crates/agent-runtime/src/swarm/mod.rs`, `swarm/llm.rs`)**:
  - Fan out programmatic `G1` scenes concurrently across up to 7 dedicated `gfx-01..0N` micro-agents via `tokio::task::JoinSet`, each authoring 1 cohesive, art-directed scene in 1–2 turns with per-scene `director_model` retry on failure.
  - Overlap Wave 1 (`razor-01` + `grade-01` + `mix-01`) in footage edits before Wave 2 (`broll-01` + `caps-01` + `gfx-01`) so color, audio, and timeline animate simultaneously from second 1.
  - Emit throttled live streaming cursor pulses and exact `timeline.track.<id>@<sec>` anchors when tools stream and commit.
- [x] **High-Velocity Multi-Cursor UI & Exact Timecode Targeting (`apps/studio-web/src/styles/globals.css`, `SwarmCursorLayer.tsx`, `StudioWorkspace.tsx`)**:
  - Process every newly arrived `op_committed`/`cursor_pulse` event in `SwarmCursorLayer` (fixing batched multi-agent event drops), remove the 3-cursor cap, support exact `timeline.track.<id>@<sec>` X-coordinate mapping on `[data-timeline-lane]`, and tighten cursor transitions (`280ms`).
- [x] **Tests, Full Verification & Docs**: Add unit tests for `ReasoningConfig` serialization, `resolve_programmatic_scenes` normalization/parsing, and run `cargo fmt`, `cargo clippy`, `cargo test`, `npm run typecheck`, `npm test`, `npm run build`.

---

## Track 12 — Deep Diff Review & Hardening Pass (5-Domain Audit)

- [x] **Swarm Concurrency, Track Locks & LLM Hygiene (`crates/agent-runtime/src/swarm/{mod.rs, llm.rs, deterministic.rs}`)**:
  - Fixed `f64::clamp(min > max)` panic and `NaN` propagation in `resolve_programmatic_scenes` + folded `> 7` trailing outro scenes cleanly instead of dropping them.
  - Made programmatic scene `clip_id` unique per run (`gfx-s{:02}-{run_tag}`), acquired a shared `TrackLockGuard` (`"gfx-swarm"`) on `"g1"` during parallel `JoinSet` fan-out, added cancellation `select!` + `abort_all()`, normalized deterministic `"gfx-fanout:"` error prefixes, moved `mix-01` to Wave 2 to eliminate `v1`/`a1` lock collisions & stale empty-spine snaps, and added child-token cancellation on fatal Wave 1 spine failure.
  - Hardened Director retry (`art_direction` + `scenes` preserved, low-effort reasoning gated, no consecutive `user` turns), stripped cross-provider `reasoning_details` on model rotation, enforced a true role tool allowlist in `validate_worker_tool_scope`, fixed `infer_target_duration_sec` decade/minute parsing, and passed normalized scene beats to `caps-01`.
  - Fixed `deterministic.rs` contiguous scene/caption grid overlaps, `Caps` fallback respecting non-empty `T1`, topic-agnostic Brand CSS-token fallback HTML, and voiceover-only `spine_dur` preference.
- [x] **OpenRouter, Advisor, Tools & QA Accuracy (`openrouter.rs`, `advisor.rs`, `tools/exec.rs`, `tools/schemas.rs`, `swarm/qa/*`)**:
  - Hardened OpenRouter SSE for null `choices`/`delta`, empty/`"error"` `finish_reason`, `set_sensitive(true)` on `AUTHORIZATION`, and cancellable body reads; re-exported `ReasoningConfig` and `CompletionTokensDetails`.
  - Fixed `ModelAdvisor` 4-tier variant lookup priority, empty-catalog stale-cache preservation, `Arc` cache storage, exact `:batch` segment matching, and STT ranking bonuses.
  - Hardened `captions.place_script` (pre-validation before deleting `T1`, sorted/de-overlapped bursts, word clamping, strict boolean `replace_existing`, CJK/RTL/abbreviation tokenization) and `gfx.place` (`"props": null` safety).
  - Fixed `qa/checks.rs` early-gap `break;` corrupting `g1` coverage, muted-track duration exclusion, `asset_id`-aware `is_full_canvas_gfx` without child pseudo-element false positives, no-op overlap auto-fix avoidance, `score <= 65` cap on hard issues, cancelled QA `passed: false`, and Vision QA reasoning/parse-error warnings + `G1`-aware sampling + secret scrubbing.
- [x] **Media Engine, Server Security & GFX Parity (`media-engine`, `server`)**:
  - Eliminated concurrent demo `.tmp` races & unbounded FFmpeg spawns (`demo_pack_lock`, unique UUID temp paths with `TempFileGuard`), memoized cached demo WAV peaks/beats, added `:eval=frame` to animated FFmpeg `eq=` filters, fixed `solaris-demo-reel` reset persistence race + `ProjectReset` WS broadcast + swarm cancellation, and hardened `sanitize_log_text` across case-insensitive `Bearer`/`Basic`, session/ticket, and AWS SigV4 tokens.
  - Stripped `"`/`'` in `gfx_render.rs` `sanitize_css_val` to prevent HTML attribute breakout, aligned Chromium `G1` viewport & `deviceScaleFactor` 1:1 with Studio preview (`gfx_design_viewport`), fixed landscape caption `y` clamp, improved `frames.rs` RGB overlay compositing & top-layer no-FFmpeg fallback, surfaced Chromium render errors in `backend.rs`, bumped WS broadcast buffer to `2048`, and logged Chromium-missing export failures.
- [x] **Frontend Studio Web & Docs Sync (`apps/studio-web`, `skills/`, `README.md`, `ARCHITECTURE.md`)**:
  - Fixed `SwarmCursorLayer` permanent freeze past the 121-event cap (`lastProcessedEventRef`), skipped appending `log_emitted` to `events`, separated `lastRealEventAt` for idle expiry, and aligned `timelineSpan` X mapping.
  - Paused timeline playback on `Source` tab switch, ignored `0×0` hidden `ResizeObserver` measurements, fixed `LogsDrawer` first-click error collapse & chronological clipboard copy + error handling, hardened `SourceMonitor` IN/OUT clamping/cleanup/reframe box, merged HTTP+WS logs cleanly in `App.tsx`, and synced `skills/`, `README.md`, and `ARCHITECTURE.md` (34 tools, overlay-only capture, full-canvas `G1` exemption).

---

## Track 13 — Persistent CDP Export Unblock, Granular Progress & Programmatic QA/Craft Fixes

- [x] **Fix CDP Chromium Keep-Alive Hang & Stream Live Overlay Export Progress (`crates/media-engine/src/gfx_render.rs`, `lib.rs`, `crates/server/src/routes/exports.rs`)**:
  - Replaced `stream.read_to_end` in `cdp_http_json` with a `Content-Length` / `Transfer-Encoding: chunked` HTTP/1.1 body reader (`read_http_response_body`) so Chromium's keep-alive DevTools HTTP server no longer triggers 5s read timeouts & CLI single-shot screenshot fallbacks (fixing exports stuck at 15% and eliminating hundreds of cold Chrome launches).
  - Enabled `optimizeForSpeed: true` on CDP `Page.captureScreenshot`, added lightweight headless startup flags, exposed `render_project_overlays_via_chromium_with_progress` (`OverlayRenderProgress`), streamed live per-frame overlay progress (`15% → 60%`) and FFmpeg compositing (`60% → 98%`) in `exports.rs`, and added an RAII `TempDirCleanupGuard` for `.tmp_<job_id>`.
- [x] **Fix Programmatic Vision QA & Scene Craft Contracts (`crates/agent-runtime/src/swarm/qa/vision.rs`, `swarm/llm.rs`, `skills/captions-and-gfx/SKILL.md`)**:
  - Added scene-settled timestamp sampling (`compute_project_sample_timestamps`) for programmatic `G1` timelines so Vision QA inspects completed hero frames instead of `0.40s` mid-stagger entrances, and updated Vision QA instructions so `programmatic_zero_asset: true` is never penalized for `0 video clips`.
  - Updated Director and parallel scene micro-agent prompts to enforce non-overlapping `9:16` vertical zones (`5vh..61vh` foreground G1, `63vh..76vh` clear for `T1` captions, full-height `100vh` ambient backdrop), explicit `<script src=...>` prohibition (`window.gsap` is pre-injected), canonical `01 / N` scene numbering without leaked asset IDs, vertical portrait-first diagram layout, fast `<= 0.35s` headline entrance, and non-duplicative caption narration.
- [x] **Fix Studio QA Panel Readability & Export Modal Stage Label (`apps/studio-web/src/components/studio/SwarmActivityPanel.tsx`, `ExportModal.tsx`)**:
  - Replaced `line-clamp-2` and `truncate` in the QA panel with wrapping `break-words leading-snug` inside a taller scrollable container (`max-h-48`) so every QA issue/warning is readable in full; stabilized `ExportModal` progress row truncation for live frame counts.
- [x] **Tests & Full Verification**: Added keep-alive/chunked CDP HTTP unit test, real persistent CDP Chromium smoke test, programmatic settled-scene QA timestamp test, and ran full Rust + TypeScript verification suite.

---

## Track 14 — Demo Removal & High-Velocity Swarm Cursor Fixes

- [x] **Delete Demo Studio Stack (`apps/studio-web`, `packages/engine-client`, `crates/server`, `crates/media-engine`)**:
  - Removed the `Open Demo Studio` CTA, `handleDemoLaunch`, and `demoMode` plumbing from `PreEditorChat.tsx`, `App.tsx`, `StudioWorkspace.tsx`, and `SwarmCursorLayer.tsx`.
  - Removed `openDemoProject()` from `@solaris/engine-client`, deleted `crates/server/src/routes/demo.rs` + `/api/v1/projects/demo` + `demo_pack_lock`, deleted `crates/media-engine/src/demo_media.rs`, and cleared cached `.solaris-data/demo_media/` and `solaris-demo-reel.json`.
- [x] **Fix & Speed Up Swarm Cursors (`SwarmCursorLayer.tsx`, `StudioWorkspace.tsx`, `globals.css`, `crates/agent-runtime/src/swarm/llm.rs`)**:
  - Fixed initial event drop when cursors auto-enable on Swarm start (stop advancing `lastProcessedEventRef` while disabled, and initialize `swarmCursorsEnabled` immediately if Swarm is already active).
  - Filtered the internal shared `"gfx-swarm"` track-lock alias out of `op_committed` cursor updates so parallel scene agents (`gfx-01..07`) no longer spawn a duplicate ghost cursor.
  - Resolved `timeline.track.<id>` anchors strictly onto `[data-timeline-lane]` (avoiding sticky track headers), added subtle live work micro-motion on streaming pulses while snapping cleanly on commits, added streaming `cursor_pulse` emissions during Director planning (`520ms`), tightened worker pulse throttles (`450ms`), reduced choreography tick interval (`450ms`), and tightened `.swarm-cursor` CSS transition to `160ms`.
