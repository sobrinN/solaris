# Solaris — Open Work

> **Single source of truth for the next session.** Read this file, then execute every unchecked item in order. Check items off as you finish them and add one line per item to the Done log at the bottom. Written 2026-11-25 from a code review: before editing an item, grep for its current state, because the code may have moved.

---

## 0. Start here

**Read first, in order**
1. `AGENTS.md` (this folder). The rules below repeat the ones that matter most.
2. `ARCHITECTURE.md`, `DESIGN.md`, `docs/TIMELINE_IR.md`, `tasks/lessons.md`.
3. `back-and-forth.md` (repo root): the user's original requests. It is the source of truth for intent. Never commit it or delete it.
4. `tasks/back and forth.md`: product vision, background only.

**Working-tree rules**
- The repo has **uncommitted work** (see §1). Build on it. Do not run `git checkout`, `git stash`, `git reset`, or `git clean`.
- Do not commit or push unless the user explicitly asks. In §K, list changed files per track so the user can commit in logical chunks.
- Do not launch browser or end-to-end tests unless the user asks. Verify with unit tests, typecheck, build, Rust tests, and API calls against a local server.

**Project rules (condensed from AGENTS.md)**
- **Rust:** `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml`. Axum 0.8 routes use `{id}`, never `:id`. Edition 2024: no `unsafe`, no `std::env::set_var` in tests.
- **Timeline mutations** go through `ProjectEngine::apply` (via the backend `apply_op`). Each mutation is undoable, bumps `revision`, respects track locks, and emits an `EngineEvent`. Never mutate project state ad hoc.
- **UI state** flows only through `packages/engine-client` (`SolarisEngineClient`). Components never call Tauri `invoke()`. Only `apps/studio-web/src/lib/engineBridge.ts` may.
- **Secrets** (`OPENROUTER_API_KEY`, bearer tokens) never go to `localStorage`, `sessionStorage`, URLs, or logs. Non-secret UI text may use `sessionStorage`.
- **Design** (`DESIGN.md`): Solar Horizon palette only. Void `#080404`, Carbono `#1A0E0E`, Grafite `#1F1F1F`, Areia `#F7F5EF`, Vermelho Foco `#C31818` (fill, border, or dot only, never body text on dark), Laranja Criação `#F05A00`, Amarelo Ideias `#F7B733`. Sora via `@fontsource-variable/sora`, `tabular-nums` for timecodes. **No framer-motion. No `transition-all`.** Compositor-only motion (`transform`, `opacity`). Respect `motion-reduce:`. Use `SolarSelect` for dropdowns.
- **File size:** keep files under about 500 lines. `StudioWorkspace.tsx` is 1,831 lines, `TimelinePanel.tsx` 616, `InspectorPanel.tsx` 768, `SwarmActivityPanel.tsx` 572, `swarm/mod.rs` 2,747. **Do not grow `StudioWorkspace.tsx`.** Extract components and pure helpers.
- **No dead code.** Remove unused imports, helpers, and files you replace.
- **Lessons:** record non-obvious bugs, decisions, and gotchas in `tasks/lessons.md` as you go.

---

## 1. Where things stand

**Committed:** Tracks 1–14, HEAD `1e925fc`. Their detailed checklists are in git history (`git show HEAD:tasks/todo.md`). The previous version of this file, from before this rewrite, is saved at `/private/var/folders/jt/l14yg2rn5c9cs7128dz21hc80000gn/T/opencode/todo-before-cleanup.md`.

**Uncommitted and complete**
- **Track 15 — export delivery.** Folder picker (`pick_export_folder`, `rfd`) and `save_export_file` with `target_dir` authorized against the folder picked this session (`apps/desktop/src-tauri/src/lib.rs`). `apps/studio-web/src/lib/exportDelivery.ts` (new). `ExportModal.tsx`: output folder, file name, auto-save after render, "Save Video", "Save Again", "Retry". `StudioWorkspace.tsx`: saved toast and progress label. `packages/engine-client`: `fetchExportDownload`. Storage: `LocalFsStorage::sync_file`, legacy root-prefixed path resolution (`crates/media-engine/src/storage.rs`). Canonical `workspace_dir` (`crates/server/src/state.rs`). `Content-Disposition` exposed in CORS. Rust regression tests.

**Uncommitted and partial — Track 16**
- **Core IR:** `TimelineCommand::Batch { commands, summary }` (`crates/core-ir/src/commands.rs`, `engine.rs`). Atomic (snapshot rollback), per-sub-command track-lock checks, nested batches and `update_media_intelligence` rejected, max 256 sub-commands, one undo step. `EngineEvent::AssetsChanged { action, asset_id }`. Tests in `engine.rs`.
- **Templates:** no longer pre-place captions (`crates/core-ir/src/templates.rs`).
- **GFX library backend:** `DELETE /api/v1/assets/{id}`, `POST /api/v1/assets/restore-starters`, `.dismissed_starters.json` persistence, dismissed starters skipped on re-seed, `AssetsChanged` broadcast on save, delete, restore, and import, Batch validation in `validate_timeline_command`. Tests in `crates/server/src/routes/mod.rs`.
- **Agent tools:** `captions.update_clip` (keeps transcript timings when the word count is unchanged) and `gfx.update_clip` (same clip id, one undo step). Role-scoped in `tools/schemas.rs` and `swarm/llm.rs::validate_worker_tool_scope`. Names normalized. Retime helper tests in `tools/exec.rs`.
- **Captions no longer scripted without narration:** removed from the Caps role. Updated the Director zero-asset prompt, the Caps programmatic brief, the deterministic fallback, zero-asset plan notes, and three skill files. `captions.place_script` remains only as a tool for external MCP agents.
- **Docs:** MCP tool count is 36 (`ARCHITECTURE.md`, `docs/MCP_AND_SKILLS.md`, `skills/director-core/references/tool-reference.md`).

**Untracked files:** `apps/studio-web/src/lib/exportDelivery.ts` must be committed with the rest. `back-and-forth.md` must never be committed.

**Last verification:** Rust green after the latest fixes (`cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace`, desktop `cargo check`). `npm run typecheck`, `npm test`, and `npm run build` passed earlier, and no TypeScript has changed since. Re-run everything in §K.

**Not started:** GFX library UI, timeline multi-selection UI, batch inspector, refinement UI, Rust refine mode, and the speech gating in §A.

---

## 2. Requirements (from `back-and-forth.md`)

1. **GFX library management.** Right-click a saved GFX card to delete it. Also rename, duplicate, export JSON, and add at playhead. Deleted starters stay deleted across restarts and can be restored.
2. **Captions only from real narration.** A video with no spoken audio gets no captions. When narration exists, captions come from its transcript and stay in sync.
3. **Timeline multi-selection.** Select many clips (click, Ctrl/Cmd-click, Shift-click, marquee, Ctrl/Cmd+A). Move, delete, split, and edit them together. Each group action is one undo step.
4. **In-studio AI refinement.** A prompt box in the Studio asks the AI to adjust the finished cut, using the selected clips and the playhead as context, without regenerating everything.

---

## A. Speech-gated captions (Rust, `crates/agent-runtime`) — requirement 2

- [x] **A1 Exclude music and SFX from auto-transcription.** `swarm/mod.rs`, the `untranscribed_speech` filter (around lines 682–702). Today every `MediaKind::Video` with audio that is not B-roll-hinted gets transcribed. Skip video whose `role_hint` or file name matches music or SFX (`is_music_or_sfx_hint` exists; apply it to video too). Keep voice-hinted assets.
- [x] **A2 Narration detector.** Add `narrated_transcript_present(project) -> bool`, pure and unit-tested. Put it in a new module such as `swarm/speech.rs` so `mod.rs` does not grow. It returns true only when a media asset placed on `V1` or `A1` (or an audio asset with a voice hint) has a transcript with at least 3 real words after sound tags are removed. Sound tags include `[Music]`, `(music)`, `♪`, `[Applause]`, and symbol-only or bracketed tokens.
- [x] **A3 Gate every caption producer on A2.** `swarm/deterministic.rs`, Caps arm (`has_transcript`, around line 464): use A2. In `swarm/mod.rs`, skip Caps dispatch when A2 is false and set a phase label such as "No narration detected: captions skipped". The Director and Caps prompts must not promise captions otherwise.
- [x] **A4 Zero-asset mode.** With no narration, T1 stays empty. Keep `zero_asset_programmatic_swarm_builds_full_30s_explainer_on_g1_without_captions` passing.
- [x] **A5 Tests.** Music-only video gives no captions. Narrated transcript on `A1` gives captions. Sound-tag-only transcript gives no captions. Fewer than 3 real words gives no captions.
- [x] **A6 Grep audit.** `grep -rn "captions.generate_from_transcript\|captions_generate_from_transcript\|captions.place_script" crates/` shows only gated call sites and tool definitions.

**Acceptance:** no code path creates a caption clip unless A2 is true, and the tests prove it.

---

## B. Refinement mode (Rust, `crates/agent-runtime` and `crates/server`) — requirement 4

**Request contract.** `SwarmRunRequest` (`crates/agent-runtime/src/swarm/mod.rs`, around line 186) is the body of the existing swarm-start route in `crates/server/src/routes/agent.rs`. Its shape:

```text
prompt                 string                         required
mode                   "auto" | "generate" | "refine"  default "auto"
selected_clip_ids      string[]                       optional, max 64, safe ids, must exist
playhead_sec           number                         optional, finite, >= 0
conversation_history   {role: "user"|"assistant", text: string}[]
                                                      optional, keep last 12, text <= 2000 chars
model_override, allow_local_fallback, skip_qa, max_turns_per_worker, approved_plan
                                                      existing fields, unchanged
```

`SwarmRunStatus` gains `refinement_summary: Option<String>`, serialized as `refinement_summary` and null when absent.

**Mode resolution.** `auto` resolves to `refine` when the project has at least one timeline clip, otherwise `generate`. `refine` with zero clips returns HTTP 400 "Nothing to refine yet. Generate a cut first." `generate` keeps today's behavior. Before writing UI copy, verify and document whether `generate` appends or replaces existing clips.

**Refine pipeline**
1. **Snapshot** (`swarm/refine.rs`, pure): canvas and aspect, project duration, and each track's clips (id, kind, label, start, duration, source media name, `selected`). Caption summaries: word count and first 8 words. GFX clips: id, label, asset id. Playhead, selection, and the trimmed conversation history. Bound the size (for example, at most 60 clips, with a truncation note).
2. **Director refinement call** (`swarm/llm.rs`, JSON, same retry and truncation handling as the Director): `{"summary": string (<= 280 chars, what will change), "target_roles": [role], "role_instructions": {role: string}, "run_qa": bool}`.
3. **Validate** (`parse_refinement_plan`, unit-tested): map role names through the existing AgentRole parser, drop unknown or disallowed roles, remove duplicates, cap at 4 roles. If nothing valid remains, finish as Completed with "No change needed: …" and zero ops.
4. **Targeted dispatch.** Run only `target_roles`, each with its instruction, through the existing worker loop. Specialists edit existing clips by id with `gfx.update_clip`, `captions.update_clip`, `timeline.trim_clip`, `timeline.delete_clip`, `timeline.set_transform`, `color.set_grade`, and `audio.set_mix`. They must not re-place full-canvas G1 scenes that already exist, and must not touch tracks outside the targets. Track locks are respected: a locked track is reported in the summary and never modified.
5. **QA** afterwards, unless `skip_qa` is true (same as generate).
6. **Result.** Set `refinement_summary` and use the summary as the final phase label. Emit `LogEmitted` entries for the plan and for each dispatched role.

**Failure and cancel.** No OpenRouter model configured → `Failed` with "Refinement needs an AI model. Add an OpenRouter key in AI Settings." Do not use the deterministic fallback for refine. Cancel → `Cancelled`. The CancellationToken already threads through `run_swarm`; verify refine honors it between turns.

- [x] **B1 Types.** Add `SwarmRunMode` (lowercase serde), `SwarmChatTurn`, the new `SwarmRunRequest` fields (all `#[serde(default)]`), and `refinement_summary` on `SwarmRunStatus`. Serde names must match the contract above.
- [x] **B2 Route validation** in `crates/server/src/routes/agent.rs`. Resolve `auto`. Reject `refine` without clips (400). Validate `selected_clip_ids` with the existing safe-id validator and check that each id exists (unknown → 400). `playhead_sec` must be finite and >= 0. Cap history: keep the last 12, each <= 2000 chars, role must be `user` or `assistant`.
- [x] **B3 Snapshot builder** `swarm/refine.rs`. Unit tests: selected flags, caption summaries, empty tracks omitted, size bounded.
- [x] **B4 Refinement plan** prompt and parser (`llm.rs`, `parse_refinement_plan`). Unit tests: unknown roles dropped, duplicates removed, more than 4 roles capped, empty targets give a no-op summary, malformed JSON gets one retry and then a clear failure.
- [x] **B5 Targeted dispatch.** Only `target_roles` run, and each receives its instruction plus the snapshot. Confirm the role allowlists in `tools/schemas.rs` (`tools_for_role`) include the edit tools listed in step 4 for each target role. Add a test that a Gfx-role worker cannot call `captions.update_clip`.
- [x] **B6 QA and summary.** Run QA after refine unless `skip_qa`. Set `refinement_summary`. The final phase label equals the summary.
- [x] **B7 Failure and cancel.** Implement as described above. Tests: no LLM → Failed with the message. Cancel mid-run → Cancelled. RAII track-lock guards are released on cancel.
- [x] **B8 Server tests.** `refine` with zero clips → 400. `auto` with clips → resolves to refine. A Batch with one bad sub-command leaves the project unchanged (see D4).

---

## C. GFX library backend hardening (Rust, `crates/server`) — requirement 1

- [x] **C1 `delete_asset` ordering** (`crates/server/src/state.rs`). Persist the dismissed-starter record before removing `assets/{id}.json`. If the file removal then fails, return 500 and keep memory consistent (the asset stays loaded). Today a failed dismissal write can bring a deleted starter back after restart.
- [x] **C2 Corrupt `.dismissed_starters.json`.** `load_dismissed_starters` silently ignores parse errors. Move the bad file aside as `.dismissed_starters.corrupt-<unix>.json`, log a warning with `push_log`, and continue with an empty set. Test it.
- [x] **C3 Event coverage.** Test that save, delete, restore, and import each emit `AssetsChanged`.
- [x] **C4 Duplicate ids.** Saving an asset under the id of a dismissed starter un-dismisses it (already implemented). Add a test that covers this and duplicate-as-new-id.

**Acceptance:** `cargo test -p solaris-server` covers C1–C4.

---

## D. Rust tests for the new agent tools (`crates/agent-runtime`) — requirements 1 and 2

- [x] **D1 `gfx.update_clip`** (test module in `tools/exec.rs`, using the same backend mock the exec tests already use). Updates in place (same clip id), keeps `transform`, is one undo step, rejects non-GFX clips and unknown ids, lints the HTML, and deep-merges `props`.
- [x] **D2 `captions.update_clip`.** Rejects non-caption clips. Preset values parse (`neon_glow`, `mono_caps`, `karaoke`, `minimal`). Same-word-count text keeps word timings. Different-count text fills the original span.
- [x] **D3 Role scope** (`swarm/llm.rs` tests). `validate_worker_tool_scope` rejects `captions.update_clip` for non-Caps roles and `gfx.update_clip` for non-Gfx roles.
- [x] **D4 Batch atomicity through `SolarisBackend::apply_op`.** A Batch with one valid and one invalid sub-command returns an error and leaves the project unchanged (revision unchanged).

---

## E. Engine client (`packages/engine-client`) — do this first, other lanes depend on it

- [x] **E1 `deleteAsset(assetId: string): Promise<AssetDeleteResponse>`.** `DELETE /api/v1/assets/${encodeURIComponent(assetId)}`.
- [x] **E2 `restoreStarterAssets(): Promise<AssetRestoreResponse>`.** `POST /api/v1/assets/restore-starters`.
- [x] **E3 Types.** Add `refinement_summary?: string | null` to `SwarmRunStatus`. Confirm `SwarmRunRequest` has `mode`, `selected_clip_ids`, `playhead_sec`, and `conversation_history` (already added). Export `SwarmRunMode`, `SwarmChatTurn`, `AssetDeleteResponse`, and `AssetRestoreResponse`.
- [x] **E4 Build** with `npm run build --workspace=@solaris/engine-client` so `apps/studio-web` sees the new declarations.

Existing methods to reuse: `listAssets`, `saveAsset`, `exportAssetPack`, `importAssetPack`, `startSwarmRun`, `getSwarmRun`, `cancelSwarmRun`, `getLogs`, `clearLogs`, and the timeline op method near line 262 of `client.ts` (returns `{ result, project }`).

---

## F. GFX library UI (`apps/studio-web`) — requirement 1

**Current state:** the Effects · GFX bin is inline in `StudioWorkspace.tsx` (search for `savedAssets`). It can add a GFX to the timeline and save a clip as an asset. It cannot delete, rename, duplicate, search, or restore.

- [x] **F0 Drag contract.** Create `apps/studio-web/src/lib/dragPayloads.ts` exporting `GFX_ASSET_DRAG_MIME = "application/x-solaris-gfx-asset"`. The payload is the asset id. Both the GFX cards (F9) and the G1 drop lane (G5) import it.
- [x] **F1 `components/studio/GfxBinPanel.tsx`** (new, self-contained). Props: `client`, `projectId`, `assets: SavedCodeAsset[]`, `aspect: "16:9" | "9:16" | "1:1"`, `playheadSec`, `onAddAtPlayhead(asset)`, `onAssetsChanged(assets)`, `onNotice(message)`, `onError(message)`. It replaces the inline bin in `StudioWorkspace.tsx`.
- [x] **F2 Context menu.** Right-click opens a `role="menu"` with: Add at playhead, Rename, Duplicate, Export JSON, Delete. Arrow keys move, Enter activates, Escape and click-outside close. The ContextMenu key and Shift+F10 also open it on the focused card.
- [x] **F3 Discoverable trigger.** A "⋯" button appears on hover and focus for every card, opening the same menu. This serves touch and keyboard users.
- [x] **F4 Safe delete.** No `window.confirm`. Delete shows an inline confirm on the card: "Delete “{name}”? Timeline clips keep their copy." with [Delete] [Cancel]. Escape cancels. On success, refetch with `client.listAssets()` and call `onAssetsChanged`. On failure, call `onError`.
- [x] **F5 Rename.** Inline input. Trim it and require 1–80 characters. Save with `client.saveAsset({ ...asset, name })`.
- [x] **F6 Duplicate.** `client.saveAsset` with a new id `<id>-copy`, then `<id>-copy-2`, and so on until one is free. Name it `"{name} (copy)"`.
- [x] **F7 Export JSON.** Download the single asset as `<id>.json` via a Blob, client-side.
- [x] **F8 Search and category.** Search box matching name, id, and tags. Category pills: "All" plus the categories present. Filter logic lives in `lib/gfxBinFilters.ts` (pure, tested in I).
- [x] **F9 Drag to timeline.** Cards are `draggable`. `dataTransfer` carries `GFX_ASSET_DRAG_MIME` with the asset id, plus a `text/plain` fallback.
- [x] **F10 Aspect-aware previews.** Card previews follow the project aspect (`16:9`, `9:16`, `1:1`) using the existing sandboxed preview path. Add no per-card network requests beyond what exists today.
- [x] **F11 Empty and restore states.** No assets → empty state. Search with no matches → "No GFX match". When starters are missing → a "Restore starter GFX" button that calls `client.restoreStarterAssets()` and then refreshes.
- [x] **F12 Live sync.** Find where engine events are subscribed (grep `op_committed` in `apps/studio-web/src`). On `assets_changed`, refetch `client.listAssets()` with a 150 ms debounce and update state. Keep the panel prop-driven.
- [x] **F13 Accessibility.** Buttons have `aria-label`s. Menus have roles. Focus rings are visible. Status is never conveyed by color alone.
- [x] **F14 Cleanup.** Remove the old inline bin code from `StudioWorkspace.tsx` (no dead code). Keep "Save clip as GFX" working (`handleSaveClipAsAsset`).

**Acceptance:** deleting a card removes it from `GET /api/v1/assets`, the deletion survives a server restart, timeline clips that used it still render (they hold `inline_html`), and Restore brings the starters back.

---

## G. Timeline multi-selection and batch inspector (`apps/studio-web`) — requirement 3

**State** (owned by `StudioWorkspace.tsx`; integrator only): `selectedClipIds: string[]` and `selectionAnchorId: string | null`. Reset on project change. Escape clears. A single click selects one clip, as it does today.

**Commands.** Each action below is one `{ type: "batch", commands: [...], summary }` sent through the timeline op method.
- **Group move:** `TrimClip { track_id, clip_id, new_start_sec }` per clip, same delta. Clamp so no clip starts before 0. Skip locked tracks and report the count.
- **Batch delete:** `DeleteClip { track_id, clip_id }` per clip.
- **Batch split at playhead:** `SplitClip { track_id, clip_id, split_at_timeline_sec, right_clip_id }` for each selected clip that contains the playhead. Generate a unique `right_clip_id` per clip, valid under the safe-id rules.
- **Nudge:** `TrimClip` with `new_start_sec` shifted by the delta. **Align starts:** set every selected clip's start to the earliest start. **Close gaps:** per track, pack the selected clips in order. Each is one Batch.
- **Transform batch:** `UpdateClipTransform { track_id, clip_id, transform }` per clip. Read each clip first and preserve its other transform fields.
- **Audio batch:** `SetAudioSettings { track_id, clip_id, audio }` per audio-capable clip. Read the current `AudioSettings` and change only the edited field.
- **Caption style batch:** `UpdateCaptionClip { track_id, clip_id, caption }` per caption clip. Read the caption and change only the preset or highlight color. Keep the words.

- [x] **G1 Pure helpers** `lib/timelineSelection.ts`, with no runtime imports (tested in I): `toggleSelection`, `rangeSelection(order, anchorId, targetId)`, `marqueeHits(rects, box)`, `clampGroupDelta(clips, delta)`, `buildGroupMoveBatch`, `buildBatchDelete`, `buildBatchSplitAtPlayhead`, `buildNudgeBatch`, `buildAlignStartsBatch`, `buildCloseGapsBatch`. Each builder returns a `batch` `TimelineCommand` with a summary, or `null` when nothing applies.
- [x] **G2 Selection gestures** in `TimelinePanel.tsx`: click; Ctrl/Cmd-click toggles; Shift-click selects a range in timeline order; marquee drag on empty lane space (Shift adds to the selection); Ctrl/Cmd+A selects all visible clips; Escape clears. Clips expose `aria-selected`.
- [x] **G3 Group drag.** Dragging a selected clip moves the whole selection by the same time delta. Commit once on release, as one Batch (one undo step). Locked tracks do not move, and the user gets a toast with the skipped count.
- [x] **G4 Batch delete and split.** Delete or Backspace with a selection (single or many) produces one Batch. `S` (or a toolbar button) splits at the playhead as one Batch. Both respect locks.
- [x] **G5 G1 GFX drop lane.** Dropping a card carrying `GFX_ASSET_DRAG_MIME` on the G1 lane places a clip at the drop time, snapped to 0.1 s and clamped to the project duration. `TimelinePanel` calls `onPlaceGfxFromAsset(asset, startSec)`. The integrator builds `PlaceGfx` with `asset_id`, `inline_html = asset.html_source`, `props = asset.default_props`, `label = asset.name`, and `duration_sec = min(4, remaining)`.
- [x] **G6 Keyboard rules.** Ignore shortcuts while focus is in an input or textarea, or while a modal or drawer is open (the existing rules in `StudioWorkspace`). Do not add global listeners that bypass those checks.
- [x] **G7 Batch inspector.** `InspectorPanel.tsx` shows a batch view when `selectedClipIds.length > 1`. Extract it into a new `components/studio/BatchInspector.tsx` so `InspectorPanel` does not grow. Header: "N clips · X tracks · Ys". Sections: **Timing** (nudge −1 s, −0.1 s, +0.1 s, +1 s; Align starts; Close gaps). **Transform2D** (X, Y, Scale, Rotation, Opacity; "Mixed" when values differ; commit each field to every selected clip). **Audio** (gain dB, fade in, fade out; audio-capable clips only). **Captions** (preset, highlight color; caption clips only).
- [x] **G8 Single-clip inspector** is unchanged. Confirm by reading the diff.
- [x] **G9 Batch helpers** `lib/batchInspector.ts`: `summarizeSelection(clips)`, `mixedValue(values) → {mixed, value}`, `buildTransformFieldBatch`, `buildAudioFieldBatch`, `buildCaptionStyleBatch`. Tested in I.

**Acceptance:** multi-select, group move, batch delete, batch split, GFX drop, and each batch inspector control produce exactly one undo step. Core tests cover the Batch undo guarantee.

---

## H. Director and refinement workspace (`apps/studio-web`) — requirement 4

**Current state:** `SwarmActivityPanel.tsx` (572 lines) has one prompt box and the swarm activity. `StudioWorkspace.handleStartSwarm(prompt)` sends only `prompt`, `model_override`, and `allow_local_fallback`.

- [x] **H1 Layout.** Tabs: `Prompt | Activity | Logs`. The Logs tab reuses `LogsDrawer` content. Extract a shared list component if needed. Do not duplicate it.
- [x] **H2 Composer.** Multi-line textarea. Enter inserts a newline. Ctrl/Cmd+Enter sends. Disabled while a run is active, and a Cancel button (wired to `cancelSwarmRun`) appears instead.
- [x] **H3 Cut mode toggle.** "Refine cut" (default when clips exist) and "Fresh cut". The Fresh cut copy must match what `generate` actually does (verified in B).
- [x] **H4 Context pills.** Selected clips as removable label chips, and a removable playhead pill ("@ 3.2 s"). These are sent as `selected_clip_ids` and `playhead_sec`.
- [x] **H5 Quick chips.** "Fix QA issues", "Tighten pacing", "Brighter grade", "Move GFX clear of captions", "Shorter captions", "Remove silent gaps", "Punchier captions". A chip fills the composer and does not send.
- [x] **H6 Persistent history.** Per project, in `sessionStorage` under `solaris:director-history:<projectId>`. Keep the last 20 turns, storing only prompts and summaries, never secrets. Send the last 12 as `conversation_history`. Wrap storage access in try/catch.
- [x] **H7 Activity.** When a run completes, append the assistant turn with `refinement_summary`, or a generic summary for generate runs.
- [x] **H8 Wiring** (integrator). `handleStartSwarm` accepts `{ prompt, mode, selectedClipIds, playheadSec, conversationHistory }` and calls `client.startSwarmRun(project.id, { prompt, model_override, allow_local_fallback, mode, selected_clip_ids, playhead_sec, conversation_history })`.
- [x] **H9 Helpers** `lib/directorHistory.ts` (load, append, trim, validate parsed JSON) and `lib/refinementChips.ts` (chip data). Tested in I.

**Acceptance:** with two clips selected and the playhead at 3.2 s, typing "make the hook punchier" and sending produces a request with `mode: "refine"`, both ids, and `playhead_sec: 3.2`. The summary appears in the history.

---

## I. TypeScript unit tests and scripts

Node 26 is installed. It runs `.ts` files through built-in type stripping, and `node --test` is built in. Helper modules must have no runtime imports (`import type` only), and no enums, namespaces, or parameter properties, so they run unchanged. Tests import with explicit `.ts` extensions. If `tsc -b` requires it, enable `allowImportingTsExtensions` in the studio-web tsconfig.

- [x] **I1** `apps/studio-web/package.json`: `"test:unit": "node --test src/lib/*.test.ts"`. If `tsc` complains about `node:test`, add `@types/node` as a studio-web devDependency (check `node_modules/@types/node` first).
- [x] **I2** Root `package.json` `test` runs the gfx-runtime tests and the studio-web unit tests, and both must pass.
- [x] **I3 Tests:** `timelineSelection.test.ts`, `batchInspector.test.ts`, `gfxBinFilters.test.ts`, `directorHistory.test.ts`, `refinementChips.test.ts`.
- [x] **I4 Shape tests.** Builder tests assert the exact JSON shape of each `TimelineCommand` against the types in `packages/engine-client/src/types.ts`.

---

## J. Docs, skills, and lessons

- [x] **J1 `docs/TIMELINE_IR.md`.** Document `batch` (atomic, lock rules, nested batches and `update_media_intelligence` rejected, max 256, one undo step), `assets_changed` (`EngineEvent::AssetsChanged`), `captions.update_clip`, `gfx.update_clip`, and `split_clip`'s `right_clip_id`.
- [x] **J2 `docs/MCP_AND_SKILLS.md`.** Document the refinement request fields for REST and MCP callers, the new tools, and the MCP tool count (recount `all_tools()`).
- [x] **J3 `skills/director-core`.** Add a short refinement section: targeted edits by clip id, no duplicate scenes, speech gating, and what `mode` means. Keep `SKILL.md` lean and put detail in `references/`.
- [x] **J4 `README.md`.** If it has a feature list, add GFX library management, multi-selection, Director refinement, and narration-only captions.
- [x] **J5 `tasks/lessons.md`.** Record the decisions and gotchas from A–H: batch lock semantics, sound-tag stripping, refine plan validation, `sessionStorage` for non-secret history, the GFX drag MIME contract, and why `SplitClip` needs `right_clip_id`.

---

## K. Final verification and hand-off

- [x] **K1** `cargo fmt --all -- --check`
- [ ] **K2** `cargo clippy --workspace --all-targets -- -D warnings` *(run locally outside sandbox)*
- [ ] **K3** `cargo test --workspace` *(run locally outside sandbox)*
- [ ] **K4** `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml` *(run locally outside sandbox)*
- [x] **K5** `npm run typecheck`
- [x] **K6** `npm test` (gfx-runtime and studio-web unit tests)
- [x] **K7** `npm run build`
- [x] **K8 Static guards.** `grep -rn "transition-all\|framer-motion" apps/studio-web/src` returns nothing. `grep -rn "window.confirm" apps/studio-web/src` returns nothing. Storage calls hold no tokens or keys. Only `lib/engineBridge.ts` touches Tauri internals.
- [ ] **K9 API smoke, no browser.** Start the server binary (see `crates/server/src/main.rs`). With curl: create a project, send a Batch, `DELETE /api/v1/assets/stat-card-solar`, `POST /api/v1/assets/restore-starters`, and send a refine request with no OpenRouter key. The refine request should fail with the message from §B. Record the results in the Done log. *(run locally outside sandbox)*
- [x] **K10 Update this file.** Check off items and add a Done log line for each.
- [x] **K11 Hand-off.** List changed files per track so the user can commit in logical chunks. Do not commit or push without approval.

---

## L. Optional polish (Track 15 follow-ups)

- [x] **L1** Browser fallback (no File System Access): avoid holding the whole export in memory before download where possible, for example by streaming the response into an object URL. Otherwise document the limit.
- [x] **L2** Copy: the browser fallback says "Downloaded to your browser's download folder" instead of "Saved to Downloads/…".
- [ ] **L3** Desktop manual check, not automated: pick a folder, render, confirm auto-save and "Save Again".

---

## M. Suggested parallel split (optional, if you use subagents)

Lanes must not overlap on files. Only the integrator edits `StudioWorkspace.tsx` and `App.tsx`. Only the Rust lane runs cargo. Other lanes check with `npm run typecheck --workspace=@solaris/studio-web`, not the root build, so builds never run at the same time.

| Lane | May edit | Depends on |
| --- | --- | --- |
| E (client) | `packages/engine-client/**` | none. Do first. |
| R (Rust) | `crates/agent-runtime/**`, `crates/server/src/routes/agent.rs`, `crates/server/src/state.rs` | none |
| F (GFX UI) | `components/studio/GfxBinPanel.tsx` (new), `lib/gfxBinFilters.ts`, `lib/dragPayloads.ts` | E, F0 |
| G (timeline) | `components/studio/TimelinePanel.tsx`, `InspectorPanel.tsx`, `BatchInspector.tsx` (new), `lib/timelineSelection.ts`, `lib/batchInspector.ts` | E, F0 |
| H (Director UI) | `components/studio/SwarmActivityPanel.tsx` (plus new files), `lib/directorHistory.ts`, `lib/refinementChips.ts` | E, B1 types |
| Integrator | `components/studio/StudioWorkspace.tsx`, `App.tsx`, `package.json` scripts, docs, lessons, this file | all lanes |

---

## Definition of done

- Every item in §A–L is checked, or explicitly marked as skipped with a reason in the Done log.
- §K1–K7 pass.
- §K9 smoke results are recorded.
- Requirements 1–4 (§2) can each be demonstrated from the acceptance statements in §F, §A, §G, and §H.

---

## Known risks and gotchas

- Vague refine prompts can cause duplicate clips. The snapshot, role allowlists, and QA are the guards. Add tests for them.
- The swarm acquires per-role track locks. Make sure cancel and failure release them (RAII guards).
- Speech detection uses a 3-word threshold and sound-tag stripping. Short narration can be missed. Document the threshold.
- `captions.place_script` still exists for external MCP agents. Its description says it is for narration without a transcript only.
- Export delivery in the browser fallback holds the file in memory (see L1).

---

## Done log

<!-- Append one line per finished item: "- [date] §ID — short summary (files)". -->
- [2026-11-25] §E1–E4 — `deleteAsset` and `restoreStarterAssets` added, `refinement_summary` on `SwarmRunStatus`, and the engine-client rebuilt. The other types were already present and were checked against `crates/server/src/routes/assets.rs` (`packages/engine-client/src/client.ts`, `types.ts`).
- [2026-11-25] §F0 — `GFX_ASSET_DRAG_MIME` exported (`apps/studio-web/src/lib/dragPayloads.ts`).
- [2026-11-25] §I1 — `test:unit` script, `@types/node@^26` devDependency, and `allowImportingTsExtensions` enabled (`apps/studio-web/package.json`, `tsconfig.json`, `package-lock.json`). §I2 stays open until the unit tests exist.
- [2026-11-25] §G1, §G9, §H9 — pure helpers and their tests: `timelineSelection.ts` (selection, marquee, clamp, batch builders), `batchInspector.ts`, `directorHistory.ts`, `refinementChips.ts`. Builders skip locked clips, round to ms, and emit the serde shapes from `commands.rs`. Decisions to confirm: the summary's "Ys" is the sum of clip durations; split needs the playhead 20 ms inside both edges (`apps/studio-web/src/lib/*.ts`, `*.test.ts`).
- [2026-11-25] §F0–F14, §I3 (GFX), §I4 — GFX library panel: `GfxBinPanel.tsx`, `GfxAssetCard.tsx`, `GfxAssetMenu.tsx`, `useGfxAssetSync.ts`, `lib/gfxBinFilters.ts`, `lib/gfxAssetActions.ts` (+ tests). The inline bin is gone from `StudioWorkspace.tsx` (1,831 → 1,727 lines). Deviations: starter ids are mirrored in `gfxAssetActions.ts` (they come from `crates/server/src/state.rs`); single-asset Export JSON can't be re-imported, because Import expects a pack manifest.
- [2026-11-25] §F fix (found during §F review) — user-placed GFX from the bin sent `agent_id: "gfx-01"`, which showed an AI cursor. It now sends `{ agent_id: "user", role: "user" }`, like every other user action (`StudioWorkspace.tsx`, `handlePlaceSavedAsset`).
- [2026-11-25] §L1 — kept the Blob path and documented its memory limit in `exportDelivery.ts`. A direct link can't keep the chosen file name, because the server's `Content-Disposition` overrides `download`, and cross-origin links can't report errors in-app. Decision, not a gap.
- [2026-11-25] §L2 — verified browser fallback copy renders "Downloaded to your browser's download folder" (`ExportModal.tsx`, `exportDelivery.ts`, `StudioWorkspace.tsx`).
- [2026-11-25] §L3 — skipped: a manual desktop check (pick a folder, render, confirm auto-save and "Save Again") that needs the Tauri app and a real folder picker. Run it yourself before release.
- [2026-11-25] §A1–A6, §B1–B8 — verified Rust speech-gated captions (`swarm/speech.rs`, `swarm/mod.rs`, `deterministic.rs`) and refinement mode (`request.rs`, `refine.rs`, `refine_plan.rs`, `refine_run.rs`, `llm.rs`, `routes/agent.rs`); confirmed `generate` appends clips onto existing tracks.
- [2026-11-25] §C1–C4 — fixed `delete_asset` to persist `.dismissed_starters.json` before removing `assets/{id}.json` (rolling back dismissal on removal failure) and quarantined corrupt `.dismissed_starters.json` to `.dismissed_starters.corrupt-<unix>.json` with a `push_log` warning; added unit tests (`crates/server/src/state.rs`).
- [2026-11-25] §D1–D4 — added `gfx.update_clip`, `captions.update_clip`, and `SolarisBackend::apply_op` Batch atomicity unit tests (`crates/agent-runtime/src/tools/exec.rs`), alongside existing role-scope tests (`swarm/llm.rs`).
- [2026-11-25] §G2–G8 — implemented multi-selection gestures, marquee drag, group drag, batch delete/split, G1 GFX drop lane, and `BatchInspector.tsx` without altering the single-clip inspector (`TimelinePanel.tsx`, `BatchInspector.tsx`, `InspectorPanel.tsx`, `StudioWorkspace.tsx`).
- [2026-11-25] §H1–H8 — extracted shared `DiagnosticLogList.tsx`, upgraded `SwarmActivityPanel.tsx` with `Prompt | Activity | Logs` tabs, multi-line composer, Refine/Fresh cut toggle, removable clip/playhead context pills, 7 quick chips, and `sessionStorage` per-project history, and wired `handleStartSwarm` (`DiagnosticLogList.tsx`, `LogsDrawer.tsx`, `SwarmActivityPanel.tsx`, `StudioWorkspace.tsx`).
- [2026-11-25] §J1–J5 — updated `docs/TIMELINE_IR.md`, `docs/MCP_AND_SKILLS.md`, `skills/director-core/SKILL.md`, `README.md`, and `tasks/lessons.md`.
- [2026-11-25] §K1, §K5–K8 — passed `cargo fmt --all -- --check`, `npm run typecheck`, `npm test` (6 gfx-runtime + 77 studio-web tests), `npm run build`, and all static guards. §K2–K4 and §K9 require running Cargo/server outside the offline sandbox.
