# Solaris — Studio UI Overhaul (Solar Horizon Pro NLE)

> Tracks 1–16 are complete and archived in git history. This file tracks the Solaris Studio UI overhaul.

---

## §1. Global CSS & Surface Tokens
- [x] Add `.solaris-panel` obsidian-carbono gradient elevation + hairline top highlight in `apps/studio-web/src/styles/globals.css`
- [x] Add `.solaris-scroll-no-x` to eliminate chunky OS horizontal scrollbars on the timeline
- [x] Upgrade `.solaris-slider` to support `--slider-track` dynamic fill gradients from neutral midpoint/zero
- [x] Add `.btn-solar-primary` for the hero Export CTA with compositor-only transitions and `prefers-reduced-motion` parity

## §2. Pure Helpers & Unit Tests
- [x] Create `apps/studio-web/src/lib/colorLooks.ts` (`COLOR_LOOK_PRESETS`, `applyColorLook`, `sliderTrackGradient`)
- [x] Create `apps/studio-web/src/lib/timelineFormat.ts` (`computeRulerTicks`, `formatCompactDuration`, `snapTimeSec`)
- [x] Create unit tests `apps/studio-web/src/lib/colorLooks.test.ts` and `apps/studio-web/src/lib/timelineFormat.test.ts`

## §3. Timeline Overhaul & Removal of Bottom Horizontal Scrollbar
- [x] Remove horizontal scrollbar at `1.0x` (`overflow-x-hidden`) and hide OS horizontal scrollbar when zoomed (`solaris-scroll-no-x` + wheel pan/zoom + `Fit` reset)
- [x] Edge-clamp ruler ticks via `computeRulerTicks` so labels never overflow `100%` width
- [x] Default `showAllTracks` to `true` with adaptive row heights (`44px` populated/active vs `26px` empty) and `heightPx` support
- [x] Upgrade transport bar (`SkipBack`, `-1s`, luminous Play/Pause, `+1s`, Snap toggle, Zoom + Fit)
- [x] Upgrade track headers (left semantic rail, WCAG-compliant mute badge) and rich layered clips (48-bar waveforms, duration badges, trim pill grips, chevron playhead)
- [x] Update `SwarmCursorLayer.tsx` bottom clamp (`vh - 54`) to clear the grounded status bar

## §4. Pro Studio Header, Resizable Timeline Split & Grounded Status Bar
- [x] Extract `apps/studio-web/src/components/studio/StudioHeader.tsx` with multi-agent Swarm Presence dot cluster and `.btn-solar-primary` Export CTA
- [x] Extract `apps/studio-web/src/components/studio/StudioStatusBar.tsx` (`h-6` grounded footer with engine state, revision, clip count, duration, shortcuts, canvas telemetry)
- [x] Update `StudioWorkspace.tsx` to use `StudioHeader`, `StudioStatusBar`, a resizable timeline splitter (`156..420px`), `h-9` Program Monitor header, and `GfxBinPanel` in the left `Assets` tab (shrinking `StudioWorkspace.tsx`)

## §5. Director Panel Craft & Assets Bin Integration
- [x] Align `SwarmActivityPanel.tsx` header to `h-9`, add `Assets` tab slot (`Prompt | Activity | Assets`), keep single `Enhance` button and automatic refine-in-place
- [x] Add live progress rail + active role pills to the swarm banner and semantic role-colored dots on op rows

## §6. Broadcast Scopes Polish
- [x] Align `BroadcastScopes.tsx` header to `h-9`, upgrade Waveform (`320x150`) & Vectorscope (`200x200`) resolution and synthetic phosphor distribution, polish Loudness meter

## §7. Pro Color Wheels, Filled Sliders, Look Presets & Inspector Polish
- [x] Upgrade `ColorWheelControl.tsx` to full-chroma radial-over-conic wheels with live magnitude readouts
- [x] Upgrade `InspectorPanel.tsx`: `h-9` header, dynamic `--slider-track` fills, double-click label reset, 6-card `Look Presets` grid, and semantic `Swarm Roles` telemetry
- [x] Align `BatchInspector.tsx` container and header styling with `InspectorPanel.tsx`

## §8. Verification, Lessons & Docs
- [x] Update `tasks/lessons.md` and `DESIGN.md`
- [x] Run full verification (`cargo fmt`, `cargo clippy`, `cargo test`, `npm run typecheck`, `npm test`, `npm run build`, static guards)

---

## Done log
- [2026-11-27] §1 — Added `.solaris-panel`, `.solaris-scroll-no-x`, `--slider-track` fill support on `.solaris-slider`, and `.btn-solar-primary` (`apps/studio-web/src/styles/globals.css`).
- [2026-11-27] §2 — Created `colorLooks.ts`, `timelineFormat.ts`, `colorLooks.test.ts`, and `timelineFormat.test.ts` (`apps/studio-web/src/lib/`).
- [2026-11-27] §3 — Overhauled `TimelinePanel.tsx` (eliminated `1.0x` horizontal scrollbar, hid zoomed OS horizontal scrollbar with wheel pan/zoom + `Fit`, edge-clamped ruler ticks, `showAllTracks` default `true` with adaptive `44px`/`26px` track heights, Snap toggle, 48-bar waveforms, trim grips, chevron playhead) and updated `SwarmCursorLayer.tsx` bottom clearance (`vh - 54`).
- [2026-11-27] §4 — Extracted `StudioHeader.tsx` and `StudioStatusBar.tsx`; wired resizable timeline splitter and `GfxBinPanel` in `StudioWorkspace.tsx` (`1,795 -> 1,738` lines).
- [2026-11-27] §5 — Polished `SwarmActivityPanel.tsx` with `h-9` header, `Assets` tab, live progress rail + role pills, and semantic role dots.
- [2026-11-27] §6 — Polished `BroadcastScopes.tsx` with `h-9` header, higher-res Waveform/Vectorscope canvases, phosphor cloud fallback, and `-14 LUFS` loudness meter.
- [2026-11-27] §7 — Upgraded `ColorWheelControl.tsx`, `InspectorPanel.tsx` (dynamic slider fills, double-click reset, 6-card Look Presets), and `BatchInspector.tsx`.
- [2026-11-27] §8 — Updated `tasks/lessons.md`, `DESIGN.md`, and ran full Rust + TypeScript + static guard verification.
