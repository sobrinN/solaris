import type { CaptionPreset, TimelineCommand } from "@solaris/engine-client";
import type { SelectedClip } from "./timelineSelection.ts";

export type TransformField = "x" | "y" | "scale" | "rotation_deg" | "opacity";
export type AudioField = "gain_db" | "fade_in_sec" | "fade_out_sec";

export interface CaptionStylePatch {
  preset?: CaptionPreset;
  highlight_color?: string;
}

export interface SelectionSummary {
  clipCount: number;
  trackCount: number;
  /** Sum of the selected clips' durations, in seconds. */
  durationSec: number;
}

export interface MixedValue<T> {
  mixed: boolean;
  value: T | null;
}

export function summarizeSelection(selection: readonly SelectedClip[]): SelectionSummary {
  return {
    clipCount: selection.length,
    trackCount: new Set(selection.map((entry) => entry.trackId)).size,
    durationSec: selection.reduce((total, entry) => total + entry.clip.duration_sec, 0),
  };
}

export function mixedValue<T>(values: readonly T[]): MixedValue<T> {
  if (values.length === 0) return { mixed: false, value: null };
  const [first] = values;
  return values.every((value) => value === first)
    ? { mixed: false, value: first }
    : { mixed: true, value: null };
}

export function buildTransformFieldBatch(
  selection: readonly SelectedClip[],
  field: TransformField,
  value: number,
): TimelineCommand | null {
  if (!isValidTransformValue(field, value)) return null;
  const commands = selection
    .filter((entry) => !entry.locked && entry.clip.transform[field] !== value)
    .map(
      ({ trackId, clip }): TimelineCommand => ({
        type: "update_clip_transform",
        track_id: trackId,
        clip_id: clip.id,
        transform: { ...clip.transform, [field]: value },
      }),
    );
  return batchOf(commands, `set ${field} on ${countOf(commands.length, "clip")}`);
}

export function buildAudioFieldBatch(
  selection: readonly SelectedClip[],
  field: AudioField,
  value: number,
): TimelineCommand | null {
  if (!isValidAudioValue(field, value)) return null;
  const commands = selection
    .filter(
      (entry) =>
        !entry.locked && entry.clip.payload.kind === "media" && entry.clip.audio[field] !== value,
    )
    .map(
      ({ trackId, clip }): TimelineCommand => ({
        type: "set_audio_settings",
        track_id: trackId,
        clip_id: clip.id,
        audio: { ...clip.audio, [field]: value },
      }),
    );
  return batchOf(commands, `set ${field} on ${countOf(commands.length, "clip")}`);
}

export function buildCaptionStyleBatch(
  selection: readonly SelectedClip[],
  patch: CaptionStylePatch,
): TimelineCommand | null {
  const color = patch.highlight_color?.trim();
  if (color === "") return null;
  const commands = selection.flatMap(({ trackId, clip, locked }): TimelineCommand[] => {
    if (locked || clip.payload.kind !== "caption") return [];
    const caption = clip.payload.caption;
    const next = {
      ...caption,
      preset: patch.preset ?? caption.preset,
      highlight_color: color ?? caption.highlight_color,
    };
    if (next.preset === caption.preset && next.highlight_color === caption.highlight_color) {
      return [];
    }
    return [{ type: "update_caption_clip", track_id: trackId, clip_id: clip.id, caption: next }];
  });
  return batchOf(commands, `set caption style on ${countOf(commands.length, "caption")}`);
}

function batchOf(commands: TimelineCommand[], summary: string): TimelineCommand | null {
  return commands.length > 0 ? { type: "batch", commands, summary } : null;
}

function countOf(count: number, noun: string): string {
  return `${count} ${noun}${count === 1 ? "" : "s"}`;
}

function isValidTransformValue(field: TransformField, value: number): boolean {
  if (!Number.isFinite(value)) return false;
  if (field === "scale") return value > 0;
  if (field === "opacity") return value >= 0 && value <= 1;
  return true;
}

function isValidAudioValue(field: AudioField, value: number): boolean {
  return Number.isFinite(value) && (field === "gain_db" || value >= 0);
}
