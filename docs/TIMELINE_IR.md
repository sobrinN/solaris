# Canonical Timeline IR Specification

`crates/core-ir` is the single source of truth for Solaris project state, timeline edits, undo/redo history, and real-time multi-agent synchronization.

---

## 1. Project & Canvas

```rust
Project {
  id: String,                    // UUID
  name: String,
  prompt: String,                // User's directing brief from PreEditorChat
  created_at: DateTime<Utc>,
  updated_at: DateTime<Utc>,
  revision: u64,                 // Monotonically incremented on every committed op / undo / redo
  canvas: CanvasConfig,          // width, height, fps_num, fps_den, aspect_preset
  brand_playbook_id: String,     // Active BrandPlaybook ID
  media_pool: Vec<MediaAsset>,   // Probed + analyzed source assets (includes has_audio, peaks, silences, LUFS)
  tracks: Vec<Track>,            // 7 canonical semantic tracks
  active_grade: ColorGrade,      // Master / FX grade baseline
  activity_log: Vec<OpLogEntry>, // Append-only capped operation history (max 200)
}
```

### Default Aspect Presets
- `Vertical9x16`: `1080 × 1920` @ `30/1` fps (default for Shorts / Reels / TikTok)
- `Landscape16x9`: `1920 × 1080` @ `30/1` fps (default for Explainers / YouTube)
- `Square1x1`: `1080 × 1080` @ `30/1` fps

---

## 2. The 7 Semantic Tracks

Every new `Project` initializes 7 ordered tracks matching professional short-form & explainer workflows:

1. `T1` (`TrackKind::Text`) — `TEXT` / `captions`: Word-synced kinetic captions.
2. `G1` (`TrackKind::Gfx`) — `GFX` / `overlays`: Saved or inline HTML/GSAP motion graphic components.
3. `V2` (`TrackKind::BRoll`) — `B-ROLL` / `inserts`: Video inserts and Ken Burns animated stills.
4. `V1` (`TrackKind::ARoll`) — `A-ROLL` / `primary`: Primary speaker spine, breath/silence trims, punch-in zooms.
5. `FX` (`TrackKind::Grade`) — `GRADE` / `adjustment`: Adjustment clips carrying Lift/Gamma/Gain & `.cube` LUTs.
6. `A1` (`TrackKind::Voice`) — `VOICE` / `dialog`: Primary voiceover / dialog waveform & LUFS target.
7. `A2` (`TrackKind::Music`) — `MUSIC` / `beat + sfx`: Background music bed (with sidechain ducking) + SFX hits.

---

## 3. Concurrency & Track-Level Locks

To let parallel specialist agents (`caps`, `gfx`, `broll`, `grade`, `mix`) edit simultaneously without corrupting timeline indices:
- `ProjectEngine` maintains `track_locks: HashMap<String, TrackLock>` (`holder_agent_id`, `role`, `acquired_at`, 300s TTL) exposed via `POST/DELETE /api/v1/projects/{id}/locks/{track_id}`.
- When a track has `track.locked == true` in the IR, all clip/grade/audio mutations on that track are rejected (until unlocked via `SetTrackFlags`).
- When a track is locked by agent `A` in `track_locks`, any `TimelineCommand` targeting that track from agent `B` (except `AgentRole::User` override or matching `holder_agent_id`) is cleanly rejected with `CoreError::TrackLocked`.
- Commands operating on `media_pool` or global `active_grade` do not block unrelated tracks, and `UpdateMediaIntelligence` is excluded from `undo_stack`/`redo_stack`.

---

## 4. Atomic `TimelineCommand` Catalog

| Command | Target Anchor (`ui_anchor`) | Behavior |
| --- | --- | --- |
| `AddMedia` | `media.pool` | Registers a `MediaAsset` in `project.media_pool` |
| `UpdateMediaIntelligence` | `media.pool.<id>` | Attaches probed metadata, peaks, silences, LUFS, proxy path (non-undoable) |
| `PlaceClip` | `timeline.track.<code_lower>` | Places a `Clip` onto a track (validates finite `duration_sec > 0` & unique ID) |
| `SplitClip` | `timeline.clip.<id>` | Splits a clip at `split_at_timeline_sec`, adjusting `source_in_sec` and partitioning `CaptionWord`s |
| `TrimClip` | `timeline.clip.<id>` | Adjusts `start_sec`, `duration_sec`, and/or `source_in_sec` |
| `DeleteClip` | `timeline.track.<code_lower>` | Removes a clip by ID |
| `UpdateClipTransform` | `program.stage` | Updates `x`, `y`, `scale`, `rotation_deg`, `opacity`, or `smart_reframe` box |
| `SetColorGrade` | `color.inspector` | Updates master `active_grade` or clip-scoped `ClipKind::Grade` Lift/Gamma/Gain, temp/tint/contrast/sat/exp, LUT |
| `UpdateCaptionClip` | `captions.inspector` | Updates caption words, preset (`NeonGlow`, `MonoCaps`, `Karaoke`, `Minimal`), font, color |
| `PlaceGfx` | `timeline.track.g1` | Instantiates a saved `SavedCodeAsset` (with typed JSON props) or inline HTML/GSAP onto `G1` |
| `UpdateGfxProps` | `program.stage` | Updates typed JSON props on an existing `G1` GFX clip |
| `SetAudioSettings` | `audio.mixer` | Updates `gain_db`, `ducking_enabled`, `duck_target_db`, fades |
| `SetTrackFlags` | `timeline.track.<code_lower>` | Toggles `muted`, `solo`, `locked` |
| `SetBrandPlaybook` | `brand.playbook` | Switches active `brand_playbook_id` |

---

## 5. Real-Time `EngineEvent` Bus

Every committed command broadcasts an `EngineEventEnvelope` over `/api/v1/ws`:
- `OpCommitted`: `{ project_id, revision, agent_id, role, command_kind, summary, ui_anchor, project }`
- `ProjectReset`: `{ project_id, revision, reason, project }` (emitted on Undo/Redo)
- `MediaAnalyzed`: `{ project_id, media_id, intelligence }`
- `TrackLockChanged`: `{ project_id, track_id, holder_agent_id, role }`
- `CursorPulse`: `{ project_id, agent_id, role, ui_anchor, label }` (allows external MCP clients or swarm workers to show live inspection focus even before committing an op)
- `ExportProgress`: `{ project_id, job }` (queued, rendering_gfx, compositing, completed, failed)
- `SwarmProgress`: `{ project_id, run }` (live multi-agent Swarm status snapshots: stage transitions, active specialist roles, fallback flag, and final QA report)
- `PlayheadSeek`: `{ project_id, time_sec, agent_id, role }` (broadcast when an agent or MCP client runs `preview.seek` so Studio Web syncs the playhead in real time)
