import { useMemo, useRef, useState } from "react";
import {
  ChevronLeft,
  ChevronRight,
  Lock,
  Magnet,
  Pause,
  Play,
  Scissors,
  SkipBack,
  Trash2,
  Volume2,
  VolumeX,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import type {
  Clip,
  MediaAsset,
  SavedCodeAsset,
  Track,
  TrackKind,
} from "@solaris/engine-client";
import { GFX_ASSET_DRAG_MIME } from "../../lib/dragPayloads";
import {
  computeRulerTicks,
  formatCompactDuration,
  snapTimeSec,
} from "../../lib/timelineFormat";
import {
  clampGroupDelta,
  marqueeHits,
  rangeSelection,
  toggleSelection,
  type MarqueeRect,
} from "../../lib/timelineSelection";

interface TimelinePanelProps {
  tracks: Track[];
  activeTrackIds?: string[];
  mediaPool?: MediaAsset[];
  savedAssets?: SavedCodeAsset[];
  playheadSec: number;
  totalDurationSec: number;
  fps?: number;
  canvasLabel?: string;
  heightPx?: number;
  isPlaying: boolean;
  selectedClipIds: string[];
  selectionAnchorId: string | null;
  onSelectionChange: (ids: string[], anchorId: string | null) => void;
  onTogglePlay: () => void;
  onSeek: (sec: number) => void;
  onSplitSelected?: () => void;
  onDeleteSelected?: () => void;
  onGroupMove?: (deltaSec: number) => void;
  onTrimOrMoveClip?: (
    trackId: string,
    clipId: string,
    patch: {
      new_start_sec?: number;
      new_duration_sec?: number;
      new_source_in_sec?: number;
    },
  ) => void;
  onDropMediaOnTrack?: (trackId: string, mediaId: string, startSec: number) => void;
  onPlaceGfxFromAsset?: (asset: SavedCodeAsset, startSec: number) => void;
  onToggleTrackFlag: (
    trackId: string,
    field: "muted" | "solo" | "locked",
    value: boolean,
  ) => void;
}

const TRACK_STRIP_COLORS: Record<TrackKind, string> = {
  text: "bg-track-t1",
  gfx: "bg-track-g1",
  b_roll: "bg-track-v2",
  a_roll: "bg-track-v1",
  grade: "bg-track-fx",
  voice: "bg-track-a1",
  music: "bg-track-a2",
};

const TRACK_CLIP_TINTS: Record<TrackKind, string> = {
  text: "bg-track-t1/[0.16] border-track-t1/40 hover:border-track-t1/70 shadow-[inset_0_1px_0_rgba(247,245,239,0.08)]",
  gfx: "bg-track-g1/[0.16] border-track-g1/40 hover:border-track-g1/70 shadow-[inset_0_1px_0_rgba(247,245,239,0.08)]",
  b_roll: "bg-track-v2/[0.16] border-track-v2/40 hover:border-track-v2/70 shadow-[inset_0_1px_0_rgba(247,245,239,0.08)]",
  a_roll: "bg-track-v1/[0.16] border-track-v1/40 hover:border-track-v1/70 shadow-[inset_0_1px_0_rgba(247,245,239,0.08)]",
  grade: "bg-track-fx/[0.18] border-track-fx/45 hover:border-track-fx/75 shadow-[inset_0_1px_0_rgba(247,245,239,0.08)]",
  voice: "bg-track-a1/[0.16] border-track-a1/40 hover:border-track-a1/70 shadow-[inset_0_1px_0_rgba(247,245,239,0.08)]",
  music: "bg-track-a2/[0.16] border-track-a2/40 hover:border-track-a2/70 shadow-[inset_0_1px_0_rgba(247,245,239,0.08)]",
};

function formatTimecode(sec: number, fps: number): string {
  const clamped = Math.max(0, Number.isFinite(sec) ? sec : 0);
  const hours = Math.floor(clamped / 3600);
  const mins = Math.floor((clamped % 3600) / 60);
  const secs = Math.floor(clamped % 60);
  const frames = Math.floor((clamped % 1) * fps);
  return `${String(hours).padStart(2, "0")}:${String(mins).padStart(2, "0")}:${String(secs).padStart(2, "0")}:${String(frames).padStart(2, "0")}`;
}

interface ClipDragPreview {
  trackId: string;
  clipId: string;
  mode: "move" | "trim-left" | "trim-right" | "group-move";
  originX: number;
  laneWidth: number;
  origStart: number;
  origDur: number;
  origSourceIn: number;
  curStart: number;
  curDur: number;
  curSourceIn: number;
  groupDeltaSec: number;
  groupMoved: boolean;
}

interface MarqueeDraft {
  originX: number;
  originY: number;
  curX: number;
  curY: number;
  additive: boolean;
  baseIds: string[];
  active: boolean;
}

function sliceWaveformPeaks(
  peaks: number[],
  mediaDurationSec: number,
  sourceInSec: number,
  clipDurationSec: number,
): number[] {
  if (peaks.length === 0) return [];
  const safeMediaDur =
    mediaDurationSec > 0.05 ? mediaDurationSec : Math.max(sourceInSec + clipDurationSec, 1);
  const startFrac = Math.max(0, Math.min(1, sourceInSec / safeMediaDur));
  const endFrac = Math.max(
    startFrac,
    Math.min(1, (sourceInSec + clipDurationSec) / safeMediaDur),
  );
  const startIdx = Math.floor(startFrac * peaks.length);
  const endIdx = Math.max(startIdx + 1, Math.ceil(endFrac * peaks.length));
  const slice = peaks.slice(startIdx, Math.min(peaks.length, endIdx));
  if (slice.length <= 48) return slice;
  const out: number[] = [];
  for (let i = 0; i < 48; i++) {
    const lo = Math.floor((i * slice.length) / 48);
    const hi = Math.max(lo + 1, Math.floor(((i + 1) * slice.length) / 48));
    let maxVal = 0;
    for (let j = lo; j < hi; j++) {
      if (slice[j] > maxVal) maxVal = slice[j];
    }
    out.push(maxVal);
  }
  return out;
}

export function TimelinePanel({
  tracks,
  activeTrackIds = [],
  mediaPool = [],
  savedAssets = [],
  playheadSec,
  totalDurationSec,
  fps = 30,
  heightPx,
  isPlaying,
  selectedClipIds,
  selectionAnchorId,
  onSelectionChange,
  onTogglePlay,
  onSeek,
  onSplitSelected,
  onDeleteSelected,
  onGroupMove,
  onTrimOrMoveClip,
  onDropMediaOnTrack,
  onPlaceGfxFromAsset,
  onToggleTrackFlag,
}: TimelinePanelProps) {
  const [zoom, setZoom] = useState(1);
  const [snapEnabled, setSnapEnabled] = useState(true);
  const [dragPreview, setDragPreview] = useState<ClipDragPreview | null>(null);
  const dragPreviewRef = useRef<ClipDragPreview | null>(null);
  const [marquee, setMarquee] = useState<MarqueeDraft | null>(null);
  const marqueeRef = useRef<MarqueeDraft | null>(null);
  const tracksContainerRef = useRef<HTMLDivElement | null>(null);
  const [dropHoverTrackId, setDropHoverTrackId] = useState<string | null>(null);
  const [emptyDropHover, setEmptyDropHover] = useState(false);
  const [showAllTracks, setShowAllTracks] = useState(true);
  const [draggingPayload, setDraggingPayload] = useState<"media" | "gfx" | null>(null);

  const activeTrackSet = useMemo(() => new Set(activeTrackIds), [activeTrackIds]);
  const visibleTracks = useMemo(() => {
    if (showAllTracks) return tracks;
    return tracks.filter((t) => {
      if (t.clips.length > 0 || activeTrackSet.has(t.id)) return true;
      if (
        draggingPayload === "media" &&
        !t.locked &&
        (t.id === "v1" || t.id === "v2" || t.id === "a1" || t.id === "a2")
      ) {
        return true;
      }
      if (draggingPayload === "gfx" && !t.locked && t.id === "g1") {
        return true;
      }
      return false;
    });
  }, [tracks, activeTrackSet, showAllTracks, draggingPayload]);
  const selectedSet = useMemo(() => new Set(selectedClipIds), [selectedClipIds]);
  const timelineClipOrder = useMemo(
    () =>
      tracks.flatMap((track) =>
        [...track.clips].sort((a, b) => a.start_sec - b.start_sec).map((clip) => clip.id),
      ),
    [tracks],
  );
  const unlockedSelectedClips = useMemo(
    () =>
      tracks.flatMap((track) =>
        track.locked ? [] : track.clips.filter((clip) => selectedSet.has(clip.id)),
      ),
    [tracks, selectedSet],
  );

  const safeFps = Math.max(1, Math.round(fps));
  const timelineSpan = totalDurationSec > 0 ? Math.max(12, Math.ceil(totalDurationSec + 2)) : 20;
  const playheadPct = Math.min(100, Math.max(0, (playheadSec / timelineSpan) * 100));
  const { ticks: rulerTicks } = useMemo(
    () => computeRulerTicks(timelineSpan, zoom),
    [timelineSpan, zoom],
  );

  const seekFromPointer = (el: HTMLDivElement, clientX: number) => {
    const rect = el.getBoundingClientRect();
    if (rect.width <= 0) return;
    const ratio = Math.max(0, Math.min(1, (clientX - rect.left) / rect.width));
    onSeek(Math.min(timelineSpan, ratio * timelineSpan));
  };

  const resetZoomFit = () => {
    setZoom(1);
    if (tracksContainerRef.current) {
      tracksContainerRef.current.scrollLeft = 0;
    }
  };

  const handleClipClick = (
    e: React.MouseEvent | React.KeyboardEvent,
    clipId: string,
  ) => {
    e.stopPropagation();
    const isToggle = e.metaKey || e.ctrlKey;
    if (e.shiftKey) {
      const anchor = selectionAnchorId ?? selectedClipIds[0] ?? clipId;
      const next = rangeSelection(timelineClipOrder, anchor, clipId);
      onSelectionChange(next, anchor);
      return;
    }
    if (isToggle) {
      const next = toggleSelection(selectedClipIds, clipId);
      onSelectionChange(next, next.length > 0 ? clipId : null);
      return;
    }
    onSelectionChange([clipId], clipId);
  };

  const beginClipDrag = (
    e: React.PointerEvent,
    track: Track,
    clip: Clip,
    mode: "move" | "trim-left" | "trim-right",
  ) => {
    if (e.button !== 0) return;
    // Let modifier clicks select without starting a drag
    if (mode === "move" && (e.metaKey || e.ctrlKey || e.shiftKey)) {
      e.stopPropagation();
      return;
    }
    const isGroupMove =
      mode === "move" &&
      Boolean(onGroupMove) &&
      selectedSet.has(clip.id) &&
      selectedClipIds.length > 1;
    if (!isGroupMove && (track.locked || !onTrimOrMoveClip)) return;
    if (isGroupMove && unlockedSelectedClips.length === 0) return;

    e.stopPropagation();
    const laneEl = (e.currentTarget as HTMLElement).closest<HTMLElement>("[data-timeline-lane]");
    const laneWidth = laneEl?.getBoundingClientRect().width ?? 0;
    if (laneWidth <= 0) return;

    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    const state: ClipDragPreview = {
      trackId: track.id,
      clipId: clip.id,
      mode: isGroupMove ? "group-move" : mode,
      originX: e.clientX,
      laneWidth,
      origStart: clip.start_sec,
      origDur: clip.duration_sec,
      origSourceIn: clip.source_in_sec,
      curStart: clip.start_sec,
      curDur: clip.duration_sec,
      curSourceIn: clip.source_in_sec,
      groupDeltaSec: 0,
      groupMoved: false,
    };
    dragPreviewRef.current = state;
    setDragPreview(state);
    if (!isGroupMove) {
      onSelectionChange([clip.id], clip.id);
    }
  };

  const updateClipDrag = (e: React.PointerEvent) => {
    const st = dragPreviewRef.current;
    if (!st) return;
    e.stopPropagation();
    const dx = e.clientX - st.originX;
    const deltaSec = (dx / st.laneWidth) * timelineSpan;

    if (st.mode === "group-move") {
      const snappedDelta = snapEnabled ? Math.round(deltaSec * 10) / 10 : deltaSec;
      const clamped = clampGroupDelta(unlockedSelectedClips, snappedDelta);
      const rounded = Math.round(clamped * 1000) / 1000;
      const updated: ClipDragPreview = {
        ...st,
        groupDeltaSec: rounded,
        groupMoved: st.groupMoved || Math.abs(dx) > 3,
      };
      dragPreviewRef.current = updated;
      setDragPreview(updated);
      return;
    }

    let nextStart = st.origStart;
    let nextDur = st.origDur;
    let nextSourceIn = st.origSourceIn;

    if (st.mode === "move") {
      nextStart = snapTimeSec(Math.max(0, st.origStart + deltaSec), snapEnabled);
    } else if (st.mode === "trim-right") {
      const trackObj = tracks.find((t) => t.id === st.trackId);
      const clipObj = trackObj?.clips.find((c) => c.id === st.clipId);
      let maxDur = Infinity;
      if (clipObj?.payload.kind === "media") {
        const mediaId = clipObj.payload.media_id;
        const asset = mediaPool.find((m) => m.id === mediaId);
        if (
          asset &&
          (asset.kind === "video" || asset.kind === "audio") &&
          asset.intelligence.duration_sec > 0
        ) {
          maxDur = Math.max(0.15, asset.intelligence.duration_sec - st.origSourceIn);
        }
      }
      const snappedEnd = snapTimeSec(
        Math.max(st.origStart + 0.15, st.origStart + st.origDur + deltaSec),
        snapEnabled,
      );
      nextDur = Math.min(maxDur, Math.max(0.15, Number((snappedEnd - st.origStart).toFixed(3))));
    } else if (st.mode === "trim-left") {
      const maxShift = st.origDur - 0.15;
      const minShift = Math.max(-st.origStart, -st.origSourceIn);
      const rawStart = Math.max(0, st.origStart + deltaSec);
      const snappedStart = snapTimeSec(rawStart, snapEnabled);
      const shift = Math.max(minShift, Math.min(maxShift, snappedStart - st.origStart));
      nextStart = st.origStart + shift;
      nextDur = st.origDur - shift;
      nextSourceIn = Math.max(0, st.origSourceIn + shift);
    }

    const updated: ClipDragPreview = {
      ...st,
      curStart: Number(nextStart.toFixed(3)),
      curDur: Number(nextDur.toFixed(3)),
      curSourceIn: Number(nextSourceIn.toFixed(3)),
    };
    dragPreviewRef.current = updated;
    setDragPreview(updated);
  };

  const finishClipDrag = (e: React.PointerEvent) => {
    const st = dragPreviewRef.current;
    if (!st) return;
    e.stopPropagation();
    if ((e.currentTarget as HTMLElement).hasPointerCapture(e.pointerId)) {
      (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    }
    dragPreviewRef.current = null;
    setDragPreview(null);

    if (st.mode === "group-move") {
      if (Math.abs(st.groupDeltaSec) > 0.01 && onGroupMove) {
        onGroupMove(st.groupDeltaSec);
      }
      return;
    }

    const changed =
      Math.abs(st.curStart - st.origStart) > 0.01 ||
      Math.abs(st.curDur - st.origDur) > 0.01 ||
      Math.abs(st.curSourceIn - st.origSourceIn) > 0.01;
    if (changed && onTrimOrMoveClip) {
      onTrimOrMoveClip(st.trackId, st.clipId, {
        new_start_sec: st.curStart,
        new_duration_sec: st.curDur,
        new_source_in_sec: st.curSourceIn,
      });
    }
  };

  const computeMarqueeSelection = (draft: MarqueeDraft) => {
    const container = tracksContainerRef.current;
    if (!container) return;
    const containerRect = container.getBoundingClientRect();
    const clipNodes = container.querySelectorAll<HTMLElement>("[data-timeline-clip-id]");
    const rects: MarqueeRect[] = [];
    clipNodes.forEach((node) => {
      const id = node.dataset.timelineClipId;
      if (!id) return;
      const r = node.getBoundingClientRect();
      rects.push({
        id,
        x: r.left - containerRect.left + container.scrollLeft,
        y: r.top - containerRect.top + container.scrollTop,
        width: r.width,
        height: r.height,
      });
    });
    const hits = marqueeHits(rects, {
      x: draft.originX,
      y: draft.originY,
      width: draft.curX - draft.originX,
      height: draft.curY - draft.originY,
    });
    const nextIds = draft.additive
      ? Array.from(new Set([...draft.baseIds, ...hits]))
      : hits;
    onSelectionChange(nextIds, nextIds[0] ?? null);
  };

  const beginLanePointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const container = tracksContainerRef.current;
    if (!container) {
      seekFromPointer(e.currentTarget, e.clientX);
      return;
    }
    const containerRect = container.getBoundingClientRect();
    const x = e.clientX - containerRect.left + container.scrollLeft;
    const y = e.clientY - containerRect.top + container.scrollTop;
    e.currentTarget.setPointerCapture(e.pointerId);
    const draft: MarqueeDraft = {
      originX: x,
      originY: y,
      curX: x,
      curY: y,
      additive: e.shiftKey,
      baseIds: selectedClipIds,
      active: false,
    };
    marqueeRef.current = draft;
    setMarquee(draft);
  };

  const updateLanePointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const draft = marqueeRef.current;
    if (!draft || dragPreviewRef.current) return;
    const container = tracksContainerRef.current;
    if (!container) return;
    const containerRect = container.getBoundingClientRect();
    const x = e.clientX - containerRect.left + container.scrollLeft;
    const y = e.clientY - containerRect.top + container.scrollTop;
    const active =
      draft.active || Math.hypot(x - draft.originX, y - draft.originY) > 4;
    const next: MarqueeDraft = { ...draft, curX: x, curY: y, active };
    marqueeRef.current = next;
    setMarquee(next);
    if (active) {
      computeMarqueeSelection(next);
    }
  };

  const finishLanePointerUp = (e: React.PointerEvent<HTMLDivElement>) => {
    const draft = marqueeRef.current;
    if (!draft) return;
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId);
    }
    marqueeRef.current = null;
    setMarquee(null);
    if (!draft.active) {
      seekFromPointer(e.currentTarget, e.clientX);
    }
  };

  const marqueeBoxStyle =
    marquee && marquee.active
      ? {
          left: Math.min(marquee.originX, marquee.curX),
          top: Math.min(marquee.originY, marquee.curY),
          width: Math.abs(marquee.curX - marquee.originX),
          height: Math.abs(marquee.curY - marquee.originY),
        }
      : null;

  return (
    <section
      data-cursor-anchor="timeline"
      style={heightPx ? { height: `${heightPx}px` } : undefined}
      className="solaris-panel flex min-h-0 flex-col overflow-hidden rounded-xl"
    >
      {/* Transport Bar */}
      <div className="flex h-9 shrink-0 items-center justify-between gap-2 border-b border-areia/[0.08] px-3">
        <div className="flex min-w-0 items-center gap-1.5">
          <button
            type="button"
            onClick={() => onSeek(0)}
            className="grid h-6 w-6 place-items-center rounded-full border border-areia/12 text-areia/75 transition-colors hover:bg-accent hover:text-areia motion-reduce:transition-none"
            aria-label="Jump to start"
            title="Jump to start"
          >
            <SkipBack className="h-3 w-3" />
          </button>
          <button
            type="button"
            onClick={() => onSeek(Math.max(0, playheadSec - 1))}
            className="grid h-6 w-6 place-items-center rounded-full border border-areia/10 text-areia/65 transition-colors hover:bg-accent hover:text-areia motion-reduce:transition-none"
            aria-label="Step back 1 second"
            title="Step back 1s"
          >
            <ChevronLeft className="h-3 w-3" />
          </button>
          <button
            type="button"
            onClick={onTogglePlay}
            className="grid h-7 w-7 place-items-center rounded-full bg-primary text-areia shadow-[0_0_14px_-2px_rgba(240,90,0,0.7)] transition-transform hover:scale-105 active:scale-95 motion-reduce:transition-none"
            aria-label={isPlaying ? "Pause" : "Play"}
            title={isPlaying ? "Pause (Space)" : "Play (Space)"}
          >
            {isPlaying ? (
              <Pause className="h-3.5 w-3.5" />
            ) : (
              <Play className="h-3.5 w-3.5 translate-x-[0.5px]" />
            )}
          </button>
          <button
            type="button"
            onClick={() => onSeek(Math.min(timelineSpan, playheadSec + 1))}
            className="grid h-6 w-6 place-items-center rounded-full border border-areia/10 text-areia/65 transition-colors hover:bg-accent hover:text-areia motion-reduce:transition-none"
            aria-label="Step forward 1 second"
            title="Step forward 1s"
          >
            <ChevronRight className="h-3 w-3" />
          </button>

          <div className="ml-2 flex items-baseline gap-1.5">
            <span className="tabular-nums text-[13px] font-semibold tracking-tight text-amarelo-ideias">
              {formatTimecode(playheadSec, safeFps)}
            </span>
            <span className="tabular-nums text-[11px] text-areia/40">
              / {formatTimecode(timelineSpan, safeFps)}
            </span>
          </div>

          {selectedClipIds.length > 0 && (
            <div className="ml-2.5 flex items-center gap-1 border-l border-areia/12 pl-2.5">
              {selectedClipIds.length > 1 && (
                <span className="tabular-nums mr-0.5 rounded-full border border-amarelo-ideias/35 bg-amarelo-ideias/15 px-2 py-0.5 text-[10px] font-medium text-amarelo-ideias">
                  {selectedClipIds.length} selected
                </span>
              )}
              {onSplitSelected && (
                <button
                  type="button"
                  onClick={onSplitSelected}
                  className="flex h-6 items-center gap-1 rounded-full border border-areia/14 bg-void/70 px-2.5 text-[10px] text-areia/85 transition-colors hover:border-laranja-criacao/50 hover:bg-accent motion-reduce:transition-none"
                  title="Split selected clip(s) at playhead (S)"
                >
                  <Scissors className="h-3 w-3 text-laranja-criacao" />
                  <span>Split</span>
                </button>
              )}
              {onDeleteSelected && (
                <button
                  type="button"
                  onClick={onDeleteSelected}
                  className="flex h-6 items-center gap-1 rounded-full border border-areia/14 bg-void/70 px-2.5 text-[10px] text-areia/85 transition-colors hover:border-vermelho-foco/60 hover:bg-vermelho-foco/20 motion-reduce:transition-none"
                  title="Delete selected clip(s) (Delete)"
                >
                  <Trash2 className="h-3 w-3 text-vermelho-foco" />
                  <span>Delete</span>
                </button>
              )}
            </div>
          )}
        </div>

        <div className="flex shrink-0 items-center gap-2 text-[11px] text-areia/60">
          <button
            type="button"
            onClick={() => setSnapEnabled((v) => !v)}
            aria-pressed={snapEnabled}
            title={snapEnabled ? "Snapping enabled (0.1s grid)" : "Snapping disabled"}
            className={`flex h-6 items-center gap-1 rounded-full border px-2 text-[10px] font-medium transition-colors motion-reduce:transition-none ${
              snapEnabled
                ? "border-amarelo-ideias/40 bg-amarelo-ideias/12 text-amarelo-ideias"
                : "border-areia/12 bg-void/65 text-areia/55 hover:bg-accent hover:text-areia"
            }`}
          >
            <Magnet className="h-3 w-3" />
            <span className="hidden sm:inline">Snap</span>
          </button>

          <div className="flex items-center gap-1 rounded-full border border-areia/12 bg-void/65 px-1.5 py-0.5">
            <button
              type="button"
              onClick={() => setZoom((z) => Math.max(1, Number((z - 0.5).toFixed(1))))}
              disabled={zoom <= 1}
              className="rounded p-0.5 text-areia/75 hover:text-areia disabled:opacity-35"
              aria-label="Zoom out timeline"
              title="Zoom out"
            >
              <ZoomOut className="h-3 w-3" />
            </button>
            <span className="tabular-nums w-8 text-center text-[10px] text-areia/85">
              {zoom.toFixed(1)}×
            </span>
            <button
              type="button"
              onClick={() => setZoom((z) => Math.min(4, Number((z + 0.5).toFixed(1))))}
              disabled={zoom >= 4}
              className="rounded p-0.5 text-areia/75 hover:text-areia disabled:opacity-35"
              aria-label="Zoom in timeline"
              title="Zoom in"
            >
              <ZoomIn className="h-3 w-3" />
            </button>
            {zoom > 1 && (
              <button
                type="button"
                onClick={resetZoomFit}
                className="ml-0.5 rounded-full border border-areia/15 bg-carbono px-1.5 py-0.5 text-[9px] font-medium text-amarelo-ideias hover:border-amarelo-ideias/50"
                title="Reset zoom to fit timeline"
              >
                Fit
              </button>
            )}
          </div>
        </div>
      </div>

      {/* Tracks */}
      <div
        ref={tracksContainerRef}
        onWheel={(e) => {
          const el = tracksContainerRef.current;
          if (!el) return;
          if (e.ctrlKey || e.metaKey) {
            e.preventDefault();
            const delta = e.deltaY < 0 ? 0.25 : -0.25;
            setZoom((z) => Math.max(1, Math.min(4, Number((z + delta).toFixed(2)))));
            return;
          }
          if (zoom > 1 && (Math.abs(e.deltaX) > 0 || e.shiftKey)) {
            const pan = Math.abs(e.deltaX) > 0 ? e.deltaX : e.deltaY;
            el.scrollLeft += pan;
          }
        }}
        onDragOver={(e) => {
          if (
            Boolean(onPlaceGfxFromAsset) &&
            e.dataTransfer.types.includes(GFX_ASSET_DRAG_MIME)
          ) {
            if (draggingPayload !== "gfx") setDraggingPayload("gfx");
          } else if (
            Boolean(onDropMediaOnTrack) &&
            e.dataTransfer.types.includes("application/x-solaris-media-id")
          ) {
            if (draggingPayload !== "media") setDraggingPayload("media");
          }
        }}
        onDragLeave={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) {
            setDraggingPayload(null);
          }
        }}
        onDrop={() => setDraggingPayload(null)}
        className={`relative flex min-h-0 flex-1 flex-col divide-y divide-areia/[0.06] overflow-y-auto solaris-scroll ${
          zoom > 1 ? "overflow-x-auto solaris-scroll-no-x" : "overflow-x-hidden"
        }`}
      >
        {/* Timecode Ruler */}
        <div className="sticky top-0 z-20 flex h-6 shrink-0 items-stretch bg-void/95 backdrop-blur-sm">
          <div className="sticky left-0 z-30 flex w-44 shrink-0 items-center justify-between border-r border-areia/10 bg-void/95 px-2.5 text-[10px] font-medium uppercase tracking-[0.12em] text-areia/50 backdrop-blur-sm">
            <span>Tracks</span>
            <button
              type="button"
              onClick={() => setShowAllTracks((v) => !v)}
              className="tabular-nums rounded-full border border-areia/10 bg-carbono/80 px-2 py-0.5 text-[9px] font-semibold tracking-normal text-areia/70 transition-colors hover:border-laranja-criacao/40 hover:bg-accent hover:text-areia motion-reduce:transition-none"
              title={
                showAllTracks
                  ? "Show only active tracks"
                  : `Show all ${tracks.length} tracks`
              }
              aria-pressed={showAllTracks}
            >
              {showAllTracks ? `All ${tracks.length}` : `${visibleTracks.length}/${tracks.length}`}
            </button>
          </div>
          <div
            style={zoom > 1 ? { minWidth: `${Math.round(zoom * 100)}%` } : undefined}
            onPointerDown={(e) => {
              if (e.button !== 0) return;
              e.currentTarget.setPointerCapture(e.pointerId);
              seekFromPointer(e.currentTarget, e.clientX);
            }}
            onPointerMove={(e) => {
              if (e.buttons === 1) {
                seekFromPointer(e.currentTarget, e.clientX);
              }
            }}
            className="relative min-w-0 flex-1 cursor-pointer select-none"
          >
            {rulerTicks.map((tick) => (
              <div
                key={`${tick.sec}-${tick.major ? "M" : "m"}`}
                className="pointer-events-none absolute top-0 bottom-0 flex flex-col justify-between"
                style={{ left: `${tick.pct}%` }}
              >
                <span
                  className={
                    tick.major
                      ? "h-2 w-px bg-areia/25"
                      : "h-1 w-px bg-areia/12"
                  }
                />
                {tick.major && tick.pct < 98.5 && (
                  <span
                    className={`tabular-nums pb-0.5 text-[9px] text-areia/45 ${
                      tick.edge === "start"
                        ? "translate-x-1"
                        : tick.edge === "end"
                          ? "-translate-x-full pr-1"
                          : "-translate-x-1/2"
                    }`}
                  >
                    {tick.sec === 0 ? "0s" : `${tick.sec}s`}
                  </span>
                )}
              </div>
            ))}
            {/* Playhead Chevron Cap */}
            <div
              className="pointer-events-none absolute top-0 bottom-0 z-10 -translate-x-1/2"
              style={{ left: `${playheadPct}%` }}
            >
              <div className="mx-auto flex flex-col items-center">
                <div className="h-[2px] w-3 rounded-t-sm bg-amarelo-ideias" />
                <div
                  className="h-2.5 w-3 bg-laranja-criacao shadow-[0_0_8px_rgba(240,90,0,0.85)]"
                  style={{ clipPath: "polygon(0 0, 100% 0, 100% 55%, 50% 100%, 0 55%)" }}
                />
              </div>
            </div>
          </div>
        </div>

        {visibleTracks.length === 0 ? (
          <div
            onDragOver={(e) => {
              const hasMedia =
                Boolean(onDropMediaOnTrack) &&
                e.dataTransfer.types.includes("application/x-solaris-media-id");
              const hasGfx =
                Boolean(onPlaceGfxFromAsset) &&
                e.dataTransfer.types.includes(GFX_ASSET_DRAG_MIME);
              if (hasMedia || hasGfx) {
                e.preventDefault();
                e.dataTransfer.dropEffect = "copy";
                setEmptyDropHover(true);
              }
            }}
            onDragLeave={() => setEmptyDropHover(false)}
            onDrop={(e) => {
              setEmptyDropHover(false);
              if (onPlaceGfxFromAsset) {
                const gfxId =
                  e.dataTransfer.getData(GFX_ASSET_DRAG_MIME) ||
                  e.dataTransfer.getData("text/plain");
                const asset = gfxId ? savedAssets.find((a) => a.id === gfxId) : undefined;
                if (asset) {
                  e.preventDefault();
                  onPlaceGfxFromAsset(asset, 0);
                  return;
                }
              }
              if (onDropMediaOnTrack) {
                const mediaId = e.dataTransfer.getData("application/x-solaris-media-id");
                if (mediaId) {
                  e.preventDefault();
                  const item = mediaPool.find((m) => m.id === mediaId);
                  const target =
                    item?.kind === "audio"
                      ? item.role_hint === "voice" || item.role_hint === "voiceover"
                        ? "a1"
                        : "a2"
                      : "v1";
                  onDropMediaOnTrack(target, mediaId, 0);
                }
              }
            }}
            className={`flex h-16 items-center justify-center px-4 text-center text-[11px] transition-colors motion-reduce:transition-none ${
              emptyDropHover
                ? "bg-laranja-criacao/15 text-areia"
                : "bg-void/35 text-areia/45"
            }`}
          >
            <span className="pointer-events-none">
              Timeline is empty — direct the swarm or drop media/GFX here to create channels.
            </span>
          </div>
        ) : null}

        {visibleTracks.map((track, trackIdx) => {
          const acceptsMediaDrop =
            Boolean(onDropMediaOnTrack) &&
            !track.locked &&
            (track.id === "v1" || track.id === "v2" || track.id === "a1" || track.id === "a2");
          const acceptsGfxDrop =
            Boolean(onPlaceGfxFromAsset) && !track.locked && track.id === "g1";
          const isPopulated =
            track.clips.length > 0 ||
            activeTrackSet.has(track.id) ||
            dropHoverTrackId === track.id;

          return (
            <div
              key={track.id}
              data-cursor-anchor={`timeline.track.${track.id}`}
              className="flex h-[26px] shrink-0 items-stretch"
            >
              {/* Track Header */}
              <div className="sticky left-0 z-20 flex w-44 shrink-0 items-center justify-between border-r border-areia/10 bg-void/95 pr-2 pl-0 backdrop-blur-sm">
                <div
                  className="flex min-w-0 items-center gap-2"
                  title={`${track.code} ${track.name} · ${track.subtitle}`}
                >
                  <span
                    className={`w-[3px] self-stretch ${
                      isPopulated ? "opacity-100" : "opacity-45"
                    } ${TRACK_STRIP_COLORS[track.kind]}`}
                  />
                  <div className="flex min-w-0 items-center gap-1.5">
                    <span
                      className={`truncate text-[10px] font-semibold tracking-[0.08em] ${
                        isPopulated ? "text-areia" : "text-areia/65"
                      }`}
                    >
                      {track.code} {track.name}
                    </span>
                    {track.clips.length > 0 && (
                      <span className="tabular-nums shrink-0 rounded bg-grafite/90 px-1 text-[9px] font-medium text-areia/60">
                        {track.clips.length}
                      </span>
                    )}
                  </div>
                </div>
                <div className="flex items-center gap-0.5">
                  <button
                    type="button"
                    onClick={() => onToggleTrackFlag(track.id, "solo", !track.solo)}
                    className={`rounded px-1.5 py-0.5 text-[9px] font-bold transition-colors motion-reduce:transition-none ${
                      track.solo
                        ? "bg-amarelo-ideias text-void"
                        : "text-areia/40 hover:bg-accent hover:text-areia"
                    }`}
                    aria-label={`Solo ${track.code}`}
                    title={`Solo ${track.code}`}
                  >
                    S
                  </button>
                  <button
                    type="button"
                    onClick={() => onToggleTrackFlag(track.id, "muted", !track.muted)}
                    className={`rounded p-1 transition-colors motion-reduce:transition-none ${
                      track.muted
                        ? "border border-vermelho-foco/55 bg-vermelho-foco/25 text-areia"
                        : "text-areia/40 hover:bg-accent hover:text-areia"
                    }`}
                    aria-label={`Mute ${track.code}`}
                    title={`Mute ${track.code}`}
                  >
                    {track.muted ? <VolumeX className="h-3 w-3" /> : <Volume2 className="h-3 w-3" />}
                  </button>
                  <button
                    type="button"
                    onClick={() => onToggleTrackFlag(track.id, "locked", !track.locked)}
                    className={`rounded p-1 transition-colors motion-reduce:transition-none ${
                      track.locked
                        ? "bg-amarelo-ideias/15 text-amarelo-ideias"
                        : "text-areia/40 hover:bg-accent hover:text-areia"
                    }`}
                    aria-label={`Lock ${track.code}`}
                    title={`Lock ${track.code}`}
                  >
                    <Lock className="h-3 w-3" />
                  </button>
                </div>
              </div>

              {/* Track Lane */}
              <div
                data-timeline-lane
                style={zoom > 1 ? { minWidth: `${Math.round(zoom * 100)}%` } : undefined}
                onDragOver={(e) => {
                  const hasMedia =
                    acceptsMediaDrop &&
                    e.dataTransfer.types.includes("application/x-solaris-media-id");
                  const hasGfx =
                    acceptsGfxDrop &&
                    e.dataTransfer.types.includes(GFX_ASSET_DRAG_MIME);
                  if (hasMedia || hasGfx) {
                    e.preventDefault();
                    e.dataTransfer.dropEffect = "copy";
                    setDropHoverTrackId(track.id);
                  }
                }}
                onDragLeave={() => {
                  if (dropHoverTrackId === track.id) setDropHoverTrackId(null);
                }}
                onDrop={(e) => {
                  setDropHoverTrackId(null);
                  const rect = e.currentTarget.getBoundingClientRect();
                  const ratio =
                    rect.width > 0
                      ? Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width))
                      : 0;
                  const dropSec = ratio * timelineSpan;

                  if (acceptsGfxDrop && onPlaceGfxFromAsset) {
                    const gfxId =
                      e.dataTransfer.getData(GFX_ASSET_DRAG_MIME) ||
                      e.dataTransfer.getData("text/plain");
                    if (gfxId) {
                      const asset = savedAssets.find((a) => a.id === gfxId);
                      if (asset) {
                        e.preventDefault();
                        const maxStart = Math.max(0, totalDurationSec);
                        const snapped = Math.min(
                          maxStart,
                          snapTimeSec(dropSec, snapEnabled),
                        );
                        onPlaceGfxFromAsset(asset, snapped);
                        return;
                      }
                    }
                  }

                  if (!acceptsMediaDrop || !onDropMediaOnTrack) return;
                  const mediaId = e.dataTransfer.getData("application/x-solaris-media-id");
                  if (!mediaId) return;
                  e.preventDefault();
                  onDropMediaOnTrack(track.id, mediaId, snapTimeSec(dropSec, snapEnabled));
                }}
                onPointerDown={beginLanePointerDown}
                onPointerMove={updateLanePointerMove}
                onPointerUp={finishLanePointerUp}
                onPointerCancel={finishLanePointerUp}
                className={`relative min-w-0 flex-1 cursor-pointer select-none transition-colors motion-reduce:transition-none ${
                  dropHoverTrackId === track.id
                    ? "bg-laranja-criacao/15"
                    : trackIdx % 2 === 0
                      ? "bg-void/55"
                      : "bg-void/38"
                }`}
              >
                {/* Vertical timecode gridlines */}
                {rulerTicks.map((tick) => {
                  if (!tick.major || tick.sec === 0) return null;
                  return (
                    <span
                      key={tick.sec}
                      aria-hidden="true"
                      className="pointer-events-none absolute top-0 bottom-0 w-px bg-areia/[0.045]"
                      style={{ left: `${tick.pct}%` }}
                    />
                  );
                })}
                {track.clips.map((clip) => {
                  const selected = selectedSet.has(clip.id);
                  const isSinglePreview =
                    dragPreview &&
                    dragPreview.mode !== "group-move" &&
                    dragPreview.trackId === track.id &&
                    dragPreview.clipId === clip.id;
                  const isGroupPreview =
                    dragPreview?.mode === "group-move" && selected && !track.locked;

                  const effectiveStart = isSinglePreview
                    ? dragPreview.curStart
                    : isGroupPreview
                      ? Math.max(0, clip.start_sec + dragPreview.groupDeltaSec)
                      : clip.start_sec;
                  const effectiveDur = isSinglePreview ? dragPreview.curDur : clip.duration_sec;
                  const effectiveSourceIn = isSinglePreview
                    ? dragPreview.curSourceIn
                    : clip.source_in_sec;

                  const leftPct = (effectiveStart / timelineSpan) * 100;
                  const widthPct = Math.max(1.8, (effectiveDur / timelineSpan) * 100);
                  const payload = clip.payload;
                  const mediaAsset =
                    payload.kind === "media"
                      ? mediaPool.find((m) => m.id === payload.media_id)
                      : undefined;
                  const rawPeaks = mediaAsset?.intelligence.waveform_peaks ?? [];
                  const slicedPeaks =
                    rawPeaks.length > 0
                      ? sliceWaveformPeaks(
                          rawPeaks,
                          mediaAsset?.intelligence.duration_sec ?? 0,
                          effectiveSourceIn,
                          effectiveDur,
                        )
                      : [];
                  const showWave =
                    slicedPeaks.length > 0 &&
                    (track.kind === "a_roll" ||
                      track.kind === "voice" ||
                      track.kind === "music");

                  return (
                    <div
                      key={clip.id}
                      role="button"
                      tabIndex={0}
                      aria-selected={selected}
                      data-timeline-clip-id={clip.id}
                      data-cursor-anchor={`timeline.clip.${clip.id}`}
                      onPointerDown={(e) => beginClipDrag(e, track, clip, "move")}
                      onPointerMove={updateClipDrag}
                      onPointerUp={finishClipDrag}
                      onPointerCancel={finishClipDrag}
                      onClick={(e) => handleClipClick(e, clip.id)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault();
                          handleClipClick(e, clip.id);
                        }
                      }}
                      title={`${clip.label} (${effectiveStart.toFixed(2)}s · ${effectiveDur.toFixed(2)}s)`}
                      className={`group absolute top-[2px] bottom-[2px] flex items-center justify-between gap-1.5 overflow-hidden rounded-[5px] border px-2 text-left text-[10px] text-areia/95 transition-colors motion-reduce:transition-none ${
                        track.locked
                          ? "cursor-not-allowed opacity-75"
                          : "cursor-grab active:cursor-grabbing"
                      } ${
                        selected
                          ? "border-amarelo-ideias bg-carbono/95 ring-1 ring-laranja-criacao/75 shadow-[0_0_12px_-3px_rgba(247,183,51,0.45)]"
                          : TRACK_CLIP_TINTS[track.kind]
                      }`}
                      style={{
                        left: `${leftPct}%`,
                        width: `${widthPct}%`,
                      }}
                    >
                      <div
                        className={`absolute inset-x-0 top-0 h-[2px] ${TRACK_STRIP_COLORS[track.kind]}`}
                      />
                      {!track.locked && onTrimOrMoveClip && (
                        <div
                          onPointerDown={(e) => beginClipDrag(e, track, clip, "trim-left")}
                          onPointerMove={updateClipDrag}
                          onPointerUp={finishClipDrag}
                          onPointerCancel={finishClipDrag}
                          title="Drag to trim clip start"
                          className="absolute top-0 bottom-0 left-0 z-10 flex w-2 cursor-ew-resize items-center justify-center"
                        >
                          <span className="h-3 w-[2.5px] rounded-full bg-amarelo-ideias/0 transition-colors group-hover:bg-amarelo-ideias/75 motion-reduce:transition-none" />
                        </div>
                      )}
                      {!track.locked && onTrimOrMoveClip && (
                        <div
                          onPointerDown={(e) => beginClipDrag(e, track, clip, "trim-right")}
                          onPointerMove={updateClipDrag}
                          onPointerUp={finishClipDrag}
                          onPointerCancel={finishClipDrag}
                          title="Drag to trim clip duration"
                          className="absolute top-0 right-0 bottom-0 z-10 flex w-2 cursor-ew-resize items-center justify-center"
                        >
                          <span className="h-3 w-[2.5px] rounded-full bg-amarelo-ideias/0 transition-colors group-hover:bg-amarelo-ideias/75 motion-reduce:transition-none" />
                        </div>
                      )}
                      {showWave && (
                        <div
                          aria-hidden="true"
                          className="pointer-events-none absolute inset-x-1 bottom-0 flex h-3 items-end gap-[1px] opacity-35"
                        >
                          {slicedPeaks.map((peak, i) => (
                            <span
                              key={i}
                              className={`flex-1 rounded-t-xs ${TRACK_STRIP_COLORS[track.kind]}`}
                              style={{
                                height: `${Math.max(15, Math.round(peak * 100))}%`,
                              }}
                            />
                          ))}
                        </div>
                      )}
                      <span className="relative truncate font-medium leading-none">
                        {clip.label}
                      </span>
                      <span className="relative tabular-nums shrink-0 text-[9px] leading-none text-areia/55">
                        {formatCompactDuration(effectiveDur)}
                      </span>
                    </div>
                  );
                })}

                {/* Playhead Line */}
                <div
                  className="pointer-events-none absolute top-0 bottom-0 z-10 w-[1.5px] bg-laranja-criacao shadow-[0_0_8px_rgba(240,90,0,0.85)]"
                  style={{ left: `${playheadPct}%` }}
                />
              </div>
            </div>
          );
        })}

        {marqueeBoxStyle && (
          <div
            aria-hidden="true"
            className="pointer-events-none absolute z-30 rounded border border-amarelo-ideias/80 bg-amarelo-ideias/15"
            style={marqueeBoxStyle}
          />
        )}
      </div>
    </section>
  );
}
