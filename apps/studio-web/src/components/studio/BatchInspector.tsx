import { useState } from "react";
import { AlignStartVertical, Lock, Minimize2, X } from "lucide-react";
import type { CaptionPreset, TimelineCommand } from "@solaris/engine-client";
import {
  buildAudioFieldBatch,
  buildCaptionStyleBatch,
  buildTransformFieldBatch,
  mixedValue,
  summarizeSelection,
  type AudioField,
  type TransformField,
} from "../../lib/batchInspector";
import {
  buildAlignStartsBatch,
  buildCloseGapsBatch,
  buildNudgeBatch,
  lockedClipCount,
  type SelectedClip,
} from "../../lib/timelineSelection";

interface BatchInspectorProps {
  selection: SelectedClip[];
  onApplyBatch: (command: TimelineCommand, lockedSkipped?: number) => void;
  onClearSelection: () => void;
}

const CAPTION_PRESETS: Array<{ id: CaptionPreset; label: string }> = [
  { id: "neon_glow", label: "Neon Glow" },
  { id: "mono_caps", label: "Mono Caps" },
  { id: "karaoke", label: "Karaoke" },
  { id: "minimal", label: "Minimal" },
];

const NUDGE_STEPS: Array<{ delta: number; label: string }> = [
  { delta: -1, label: "−1s" },
  { delta: -0.1, label: "−0.1s" },
  { delta: 0.1, label: "+0.1s" },
  { delta: 1, label: "+1s" },
];

const SLIDER_COMMIT_KEYS = new Set([
  "ArrowLeft",
  "ArrowRight",
  "ArrowUp",
  "ArrowDown",
  "Home",
  "End",
  "PageUp",
  "PageDown",
]);

export function BatchInspector({
  selection,
  onApplyBatch,
  onClearSelection,
}: BatchInspectorProps) {
  const [transformDrafts, setTransformDrafts] = useState<Partial<Record<TransformField, number>>>(
    {},
  );
  const [audioDrafts, setAudioDrafts] = useState<Partial<Record<AudioField, number>>>({});
  const [captionColorDraft, setCaptionColorDraft] = useState<string | null>(null);

  const summary = summarizeSelection(selection);
  const lockedCount = lockedClipCount(selection);

  const audioClips = selection.filter((entry) => entry.clip.payload.kind === "media");
  const captionClips = selection.filter((entry) => entry.clip.payload.kind === "caption");

  const dispatchBatch = (cmd: TimelineCommand | null) => {
    if (!cmd) return;
    onApplyBatch(cmd, lockedCount);
  };

  const commitTransformField = (field: TransformField, value: number) => {
    setTransformDrafts((prev) => {
      const next = { ...prev };
      delete next[field];
      return next;
    });
    dispatchBatch(buildTransformFieldBatch(selection, field, value));
  };

  const commitAudioField = (field: AudioField, value: number) => {
    setAudioDrafts((prev) => {
      const next = { ...prev };
      delete next[field];
      return next;
    });
    dispatchBatch(buildAudioFieldBatch(selection, field, value));
  };

  const captionPresetMixed = mixedValue(
    captionClips.map((entry) =>
      entry.clip.payload.kind === "caption" ? entry.clip.payload.caption.preset : "minimal",
    ),
  );
  const captionColorMixed = mixedValue(
    captionClips.map((entry) =>
      entry.clip.payload.kind === "caption" ? entry.clip.payload.caption.highlight_color : "#F7B733",
    ),
  );

  const effectiveCaptionColor =
    captionColorDraft ??
    (captionColorMixed.value && /^#[0-9a-fA-F]{6}$/.test(captionColorMixed.value)
      ? captionColorMixed.value
      : "#F7B733");

  return (
    <aside
      aria-label="Batch clip inspector"
      className="flex h-full flex-col gap-3 overflow-y-auto rounded-2xl border border-areia/10 bg-carbono/70 p-3 text-xs"
    >
      {/* Header */}
      <div className="flex items-start justify-between gap-2 border-b border-areia/10 pb-2.5">
        <div className="min-w-0">
          <div className="text-[10px] font-semibold uppercase tracking-[0.14em] text-amarelo-ideias">
            Batch Edit
          </div>
          <div className="mt-0.5 tabular-nums text-xs font-medium text-areia">
            {summary.clipCount} clips · {summary.trackCount}{" "}
            {summary.trackCount === 1 ? "track" : "tracks"} · {summary.durationSec.toFixed(1)}s
          </div>
        </div>
        <button
          type="button"
          onClick={onClearSelection}
          aria-label="Clear selection"
          title="Clear selection (Escape)"
          className="grid h-6 w-6 shrink-0 place-items-center rounded-full border border-areia/14 text-areia/70 hover:bg-accent hover:text-areia"
        >
          <X className="h-3 w-3" />
        </button>
      </div>

      {lockedCount > 0 && (
        <div className="flex items-center gap-1.5 rounded-lg border border-amarelo-ideias/30 bg-amarelo-ideias/10 px-2.5 py-1.5 text-[10px] text-areia/85">
          <Lock className="h-3 w-3 shrink-0 text-amarelo-ideias" />
          <span>
            {lockedCount} locked {lockedCount === 1 ? "clip" : "clips"} will stay unchanged.
          </span>
        </div>
      )}

      {/* Timing */}
      <div className="space-y-2 rounded-xl border border-areia/12 bg-void/70 p-2.5">
        <div className="text-[10px] font-semibold uppercase tracking-[0.14em] text-areia/60">
          Timing
        </div>
        <div className="grid grid-cols-4 gap-1">
          {NUDGE_STEPS.map(({ delta, label }) => (
            <button
              key={label}
              type="button"
              onClick={() => dispatchBatch(buildNudgeBatch(selection, delta))}
              className="tabular-nums rounded-lg border border-areia/14 bg-carbono px-1.5 py-1 text-[10px] text-areia/85 transition-colors hover:border-laranja-criacao/55 hover:bg-accent motion-reduce:transition-none"
            >
              {label}
            </button>
          ))}
        </div>
        <div className="grid grid-cols-2 gap-1.5 pt-0.5">
          <button
            type="button"
            onClick={() => dispatchBatch(buildAlignStartsBatch(selection))}
            className="flex items-center justify-center gap-1 rounded-lg border border-areia/14 bg-carbono px-2 py-1.5 text-[10px] text-areia/85 transition-colors hover:border-amarelo-ideias/55 hover:bg-accent motion-reduce:transition-none"
          >
            <AlignStartVertical className="h-3 w-3 text-amarelo-ideias" />
            <span>Align starts</span>
          </button>
          <button
            type="button"
            onClick={() => dispatchBatch(buildCloseGapsBatch(selection))}
            className="flex items-center justify-center gap-1 rounded-lg border border-areia/14 bg-carbono px-2 py-1.5 text-[10px] text-areia/85 transition-colors hover:border-laranja-criacao/55 hover:bg-accent motion-reduce:transition-none"
          >
            <Minimize2 className="h-3 w-3 text-laranja-criacao" />
            <span>Close gaps</span>
          </button>
        </div>
      </div>

      {/* Transform2D */}
      <div className="space-y-2 rounded-xl border border-areia/12 bg-void/70 p-2.5">
        <div className="text-[10px] font-semibold uppercase tracking-[0.14em] text-areia/60">
          Transform2D
        </div>
        <div className="grid grid-cols-2 gap-1.5 text-[10px]">
          {(
            [
              ["x", "Pos X", -0.5, 0.5, 0.02, 0],
              ["y", "Pos Y", -0.5, 0.5, 0.02, 0],
              ["scale", "Scale", 0.2, 2.5, 0.02, 1],
              ["rotation_deg", "Rot °", -180, 180, 1, 0],
              ["opacity", "Opacity", 0, 1, 0.02, 1],
            ] as const
          ).map(([field, label, min, max, step, fallback]) => {
            const state = mixedValue(selection.map((entry) => entry.clip.transform[field]));
            const draft = transformDrafts[field];
            const numericVal = draft ?? state.value ?? fallback;
            const displayLabel =
              draft !== undefined
                ? field === "rotation_deg"
                  ? `${Math.round(draft)}°`
                  : draft.toFixed(2)
                : state.mixed
                  ? "Mixed"
                  : state.value !== null
                    ? field === "rotation_deg"
                      ? `${Math.round(state.value)}°`
                      : state.value.toFixed(2)
                    : "—";

            return (
              <label key={field} className="flex flex-col gap-0.5">
                <div className="flex justify-between text-areia/55">
                  <span>{label}</span>
                  <span
                    className={`tabular-nums ${
                      state.mixed && draft === undefined ? "text-amarelo-ideias/80" : "text-areia/80"
                    }`}
                  >
                    {displayLabel}
                  </span>
                </div>
                <input
                  type="range"
                  aria-label={`Batch ${label}`}
                  min={min}
                  max={max}
                  step={step}
                  value={Math.max(min, Math.min(max, numericVal))}
                  onChange={(e) => {
                    const val = parseFloat(e.target.value);
                    if (!Number.isFinite(val)) return;
                    setTransformDrafts((prev) => ({ ...prev, [field]: val }));
                  }}
                  onPointerUp={(e) => {
                    const val = parseFloat(e.currentTarget.value);
                    if (Number.isFinite(val)) commitTransformField(field, val);
                  }}
                  onKeyUp={(e) => {
                    if (!SLIDER_COMMIT_KEYS.has(e.key)) return;
                    const val = parseFloat(e.currentTarget.value);
                    if (Number.isFinite(val)) commitTransformField(field, val);
                  }}
                  onBlur={() => {
                    if (draft !== undefined) commitTransformField(field, draft);
                  }}
                  className="h-1 w-full cursor-pointer accent-laranja-criacao"
                />
              </label>
            );
          })}
        </div>
      </div>

      {/* Audio (media clips only) */}
      {audioClips.length > 0 && (
        <div className="space-y-2 rounded-xl border border-track-a1/30 bg-void/70 p-2.5">
          <div className="flex items-center justify-between text-[10px] font-semibold uppercase tracking-[0.14em] text-track-a1">
            <span>Audio</span>
            <span className="tabular-nums font-normal normal-case text-areia/55">
              {audioClips.length} {audioClips.length === 1 ? "clip" : "clips"}
            </span>
          </div>
          <div className="space-y-1.5 text-[10px]">
            {(
              [
                ["gain_db", "Gain dB", -48, 12, 0.5, 0],
                ["fade_in_sec", "Fade In", 0, 3, 0.05, 0],
                ["fade_out_sec", "Fade Out", 0, 3, 0.05, 0],
              ] as const
            ).map(([field, label, min, max, step, fallback]) => {
              const state = mixedValue(audioClips.map((entry) => entry.clip.audio[field]));
              const draft = audioDrafts[field];
              const numericVal = draft ?? state.value ?? fallback;
              const displayLabel =
                draft !== undefined
                  ? draft.toFixed(1)
                  : state.mixed
                    ? "Mixed"
                    : state.value !== null
                      ? state.value.toFixed(1)
                      : "—";

              return (
                <label key={field} className="flex items-center gap-2">
                  <span className="w-14 text-areia/60">{label}</span>
                  <input
                    type="range"
                    aria-label={`Batch ${label}`}
                    min={min}
                    max={max}
                    step={step}
                    value={Math.max(min, Math.min(max, numericVal))}
                    onChange={(e) => {
                      const val = parseFloat(e.target.value);
                      if (!Number.isFinite(val)) return;
                      setAudioDrafts((prev) => ({ ...prev, [field]: val }));
                    }}
                    onPointerUp={(e) => {
                      const val = parseFloat(e.currentTarget.value);
                      if (Number.isFinite(val)) commitAudioField(field, val);
                    }}
                    onKeyUp={(e) => {
                      if (!SLIDER_COMMIT_KEYS.has(e.key)) return;
                      const val = parseFloat(e.currentTarget.value);
                      if (Number.isFinite(val)) commitAudioField(field, val);
                    }}
                    onBlur={() => {
                      if (draft !== undefined) commitAudioField(field, draft);
                    }}
                    className="h-1 flex-1 cursor-pointer accent-amarelo-ideias"
                  />
                  <span
                    className={`w-10 text-right tabular-nums ${
                      state.mixed && draft === undefined ? "text-amarelo-ideias/80" : "text-areia/80"
                    }`}
                  >
                    {displayLabel}
                  </span>
                </label>
              );
            })}
          </div>
        </div>
      )}

      {/* Captions (caption clips only) */}
      {captionClips.length > 0 && (
        <div className="space-y-2 rounded-xl border border-track-t1/35 bg-void/70 p-2.5">
          <div className="flex items-center justify-between">
            <span className="text-[10px] font-semibold uppercase tracking-[0.14em] text-track-t1">
              Captions ({captionClips.length})
            </span>
            <label className="flex items-center gap-1.5 text-[10px] text-areia/65">
              {captionColorMixed.mixed && !captionColorDraft && (
                <span className="text-amarelo-ideias/80">Mixed</span>
              )}
              <input
                type="color"
                aria-label="Batch caption highlight color"
                title="Highlight color"
                value={effectiveCaptionColor}
                onChange={(e) => {
                  const nextColor = e.target.value;
                  setCaptionColorDraft(nextColor);
                  dispatchBatch(
                    buildCaptionStyleBatch(selection, { highlight_color: nextColor }),
                  );
                }}
                onBlur={() => setCaptionColorDraft(null)}
                className="h-4 w-5 cursor-pointer rounded border-0 bg-transparent p-0"
              />
            </label>
          </div>
          <div className="grid grid-cols-2 gap-1.5">
            {CAPTION_PRESETS.map((preset) => {
              const active = !captionPresetMixed.mixed && captionPresetMixed.value === preset.id;
              return (
                <button
                  key={preset.id}
                  type="button"
                  onClick={() =>
                    dispatchBatch(buildCaptionStyleBatch(selection, { preset: preset.id }))
                  }
                  className={`rounded-xl border px-2.5 py-1.5 text-left text-[11px] transition-colors motion-reduce:transition-none ${
                    active
                      ? "border-amarelo-ideias/60 bg-accent text-areia"
                      : "border-areia/10 bg-carbono/80 text-areia/75 hover:bg-accent"
                  }`}
                >
                  {preset.label}
                </button>
              );
            })}
          </div>
        </div>
      )}
    </aside>
  );
}
