import type { Clip, TimelineCommand, Track } from "@solaris/engine-client";

export interface SelectedClip {
  trackId: string;
  locked: boolean;
  clip: Clip;
}

export interface MarqueeRect {
  id: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface MarqueeBox {
  x: number;
  y: number;
  width: number;
  height: number;
}

// Same margin as the single-clip split guard in StudioWorkspace, so a split never leaves a sliver.
const SPLIT_EDGE_MARGIN_SEC = 0.02;
const SAFE_ID_PATTERN = /^[A-Za-z0-9_-]{1,64}$/;

export function toggleSelection(selectedIds: readonly string[], clipId: string): string[] {
  return selectedIds.includes(clipId)
    ? selectedIds.filter((id) => id !== clipId)
    : [...selectedIds, clipId];
}

export function rangeSelection(
  order: readonly string[],
  anchorId: string,
  targetId: string,
): string[] {
  const target = order.indexOf(targetId);
  if (target === -1) return [];
  const anchor = order.indexOf(anchorId);
  if (anchor === -1) return [targetId];
  return order.slice(Math.min(anchor, target), Math.max(anchor, target) + 1);
}

export function marqueeHits(rects: readonly MarqueeRect[], box: MarqueeBox): string[] {
  const left = Math.min(box.x, box.x + box.width);
  const right = Math.max(box.x, box.x + box.width);
  const top = Math.min(box.y, box.y + box.height);
  const bottom = Math.max(box.y, box.y + box.height);
  return rects
    .filter(
      (rect) =>
        rect.x < right &&
        rect.x + rect.width > left &&
        rect.y < bottom &&
        rect.y + rect.height > top,
    )
    .map((rect) => rect.id);
}

export function clampGroupDelta(
  clips: readonly Pick<Clip, "start_sec">[],
  deltaSec: number,
): number {
  return Math.max(deltaSec, -earliestStart(clips));
}

/** Clips that the builders skip because their track is locked. */
export function lockedClipCount(selection: readonly SelectedClip[]): number {
  return selection.filter((entry) => entry.locked).length;
}

export function resolveSelection(
  tracks: readonly Track[],
  selectedIds: readonly string[],
): SelectedClip[] {
  const selected = new Set(selectedIds);
  return tracks.flatMap((track) =>
    track.clips
      .filter((clip) => selected.has(clip.id))
      .sort((a, b) => a.start_sec - b.start_sec)
      .map((clip) => ({ trackId: track.id, locked: track.locked, clip })),
  );
}

export function buildGroupMoveBatch(
  selection: readonly SelectedClip[],
  deltaSec: number,
): TimelineCommand | null {
  return buildMoveBatch(selection, deltaSec, "moved");
}

export function buildNudgeBatch(
  selection: readonly SelectedClip[],
  deltaSec: number,
): TimelineCommand | null {
  return buildMoveBatch(selection, deltaSec, "nudged");
}

export function buildBatchDelete(selection: readonly SelectedClip[]): TimelineCommand | null {
  const clips = unlocked(selection);
  return batch(
    clips.map(
      ({ trackId, clip }): TimelineCommand => ({
        type: "delete_clip",
        track_id: trackId,
        clip_id: clip.id,
      }),
    ),
    `deleted ${countOf(clips.length, "clip")}`,
  );
}

/** `newClipId` runs once per split. It must return a unique id that passes the safe-id rule. */
export function buildBatchSplitAtPlayhead(
  selection: readonly SelectedClip[],
  playheadSec: number,
  newClipId: () => string,
): TimelineCommand | null {
  if (!Number.isFinite(playheadSec)) return null;
  const at = roundSec(playheadSec);
  const clips = unlocked(selection).filter(
    ({ clip }) =>
      clip.start_sec + SPLIT_EDGE_MARGIN_SEC < at &&
      at < clip.start_sec + clip.duration_sec - SPLIT_EDGE_MARGIN_SEC,
  );
  return batch(
    clips.map(
      ({ trackId, clip }): TimelineCommand => ({
        type: "split_clip",
        track_id: trackId,
        clip_id: clip.id,
        split_at_timeline_sec: at,
        right_clip_id: requireSafeId(newClipId()),
      }),
    ),
    `split ${countOf(clips.length, "clip")} at ${at.toFixed(2)}s`,
  );
}

/** The target is the earliest start of the whole selection, locked clips included. */
export function buildAlignStartsBatch(selection: readonly SelectedClip[]): TimelineCommand | null {
  if (selection.length === 0) return null;
  const target = roundSec(earliestStart(selection.map((entry) => entry.clip)));
  const clips = unlocked(selection).filter(({ clip }) => roundSec(clip.start_sec) !== target);
  return batch(
    clips.map(({ trackId, clip }) => trimStart(trackId, clip.id, target)),
    `aligned ${countOf(clips.length, "clip")} to ${target.toFixed(2)}s`,
  );
}

export function buildCloseGapsBatch(selection: readonly SelectedClip[]): TimelineCommand | null {
  const commands = [...groupByTrack(unlocked(selection)).values()].flatMap(packTrack);
  return batch(commands, `closed gaps by moving ${countOf(commands.length, "clip")}`);
}

function buildMoveBatch(
  selection: readonly SelectedClip[],
  deltaSec: number,
  verb: string,
): TimelineCommand | null {
  const clips = unlocked(selection);
  if (!Number.isFinite(deltaSec) || clips.length === 0) return null;
  const delta = clampGroupDelta(clips.map((entry) => entry.clip), deltaSec);
  if (roundSec(delta) === 0) return null;
  return batch(
    clips.map(({ trackId, clip }) => trimStart(trackId, clip.id, roundSec(clip.start_sec + delta))),
    `${verb} ${countOf(clips.length, "clip")} by ${signedSeconds(roundSec(delta))}`,
  );
}

function packTrack(entries: SelectedClip[]): TimelineCommand[] {
  const ordered = [...entries].sort((a, b) => a.clip.start_sec - b.clip.start_sec);
  const commands: TimelineCommand[] = [];
  let cursor = roundSec(ordered[0].clip.start_sec);
  for (const { trackId, clip } of ordered) {
    if (roundSec(clip.start_sec) !== cursor) commands.push(trimStart(trackId, clip.id, cursor));
    cursor = roundSec(cursor + clip.duration_sec);
  }
  return commands;
}

function groupByTrack(selection: readonly SelectedClip[]): Map<string, SelectedClip[]> {
  const groups = new Map<string, SelectedClip[]>();
  for (const entry of selection) {
    groups.set(entry.trackId, [...(groups.get(entry.trackId) ?? []), entry]);
  }
  return groups;
}

/** A Batch fails as a whole if any sub-command hits a locked track, so skip them here. */
function unlocked(selection: readonly SelectedClip[]): SelectedClip[] {
  return selection.filter((entry) => !entry.locked);
}

function batch(commands: TimelineCommand[], summary: string): TimelineCommand | null {
  return commands.length > 0 ? { type: "batch", commands, summary } : null;
}

function trimStart(trackId: string, clipId: string, startSec: number): TimelineCommand {
  return { type: "trim_clip", track_id: trackId, clip_id: clipId, new_start_sec: startSec };
}

function earliestStart(clips: readonly Pick<Clip, "start_sec">[]): number {
  return clips.reduce((min, clip) => Math.min(min, clip.start_sec), Infinity);
}

function requireSafeId(id: string): string {
  if (!SAFE_ID_PATTERN.test(id)) throw new RangeError(`'${id}' is not a safe id`);
  return id;
}

function roundSec(value: number): number {
  return Math.round(value * 1000) / 1000;
}

function countOf(count: number, noun: string): string {
  return `${count} ${noun}${count === 1 ? "" : "s"}`;
}

function signedSeconds(value: number): string {
  return `${value > 0 ? "+" : ""}${value.toFixed(2)}s`;
}
