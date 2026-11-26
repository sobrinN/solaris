import { useEffect, useMemo, useRef, useState } from "react";
import { AlertTriangle, BookmarkPlus } from "lucide-react";
import type {
  AudioSettings,
  CaptionBlock,
  CaptionPreset,
  Clip,
  ColorGrade,
  SavedCodeAsset,
  TimelineCommand,
  Transform2D,
} from "@solaris/engine-client";
import { extractPropFields, lintGfxHtml } from "@solaris/gfx-runtime";
import type { SelectedClip } from "../../lib/timelineSelection";
import { BatchInspector } from "./BatchInspector";
import { ColorWheelControl } from "./ColorWheelControl";

interface InspectorPanelProps {
  grade: ColorGrade;
  gradeTargetLabel?: string;
  hasBrandLut: boolean;
  onPreviewGrade: (next: ColorGrade) => void;
  onCommitGrade: (next: ColorGrade) => void;
  selectedClip: { trackId: string; clip: Clip } | null;
  selectedClips?: SelectedClip[];
  onApplyBatchCommand?: (command: TimelineCommand, lockedSkipped?: number) => void;
  onClearSelection?: () => void;
  activeCaptionPreset?: CaptionPreset | null;
  savedAssets: SavedCodeAsset[];
  onUpdateGfxProps: (
    trackId: string,
    clipId: string,
    props: Record<string, unknown>,
  ) => void;
  onUpdateClipTransform?: (
    trackId: string,
    clipId: string,
    transform: Transform2D,
  ) => void;
  onUpdateClipAudio?: (
    trackId: string,
    clipId: string,
    audio: AudioSettings,
  ) => void;
  onUpdateCaptionClip?: (
    trackId: string,
    clipId: string,
    caption: CaptionBlock,
  ) => void;
  onUpdateCaptionPreset?: (preset: CaptionPreset) => void;
  onSaveClipAsAsset: (clip: Clip) => Promise<void>;
  swarmRoleCounts: Record<string, number>;
}

const CAPTION_PRESETS: Array<{ id: CaptionPreset; label: string }> = [
  { id: "neon_glow", label: "Neon Glow" },
  { id: "mono_caps", label: "Mono Caps" },
  { id: "karaoke", label: "Karaoke" },
  { id: "minimal", label: "Minimal" },
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

export function InspectorPanel({
  grade,
  gradeTargetLabel = "Master Grade",
  hasBrandLut,
  onPreviewGrade,
  onCommitGrade,
  selectedClip,
  selectedClips,
  onApplyBatchCommand,
  onClearSelection,
  activeCaptionPreset,
  savedAssets,
  onUpdateGfxProps,
  onUpdateClipTransform,
  onUpdateClipAudio,
  onUpdateCaptionClip,
  onUpdateCaptionPreset,
  onSaveClipAsAsset,
  swarmRoleCounts,
}: InspectorPanelProps) {
  const [captionPreset, setCaptionPreset] = useState<CaptionPreset>(
    activeCaptionPreset ?? "neon_glow",
  );
  const [inspectorTab, setInspectorTab] = useState<"color" | "captions" | "clip">("color");
  const [savingAsset, setSavingAsset] = useState(false);
  const [localProps, setLocalProps] = useState<Record<string, unknown>>({});
  const [captionTextDraft, setCaptionTextDraft] = useState("");
  const [captionColorDraft, setCaptionColorDraft] = useState("#F7B733");
  const [transformDraft, setTransformDraft] = useState<Transform2D | null>(null);
  const [audioDraft, setAudioDraft] = useState<AudioSettings | null>(null);
  const propDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const captionColorDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastPropsClipIdRef = useRef<string | null>(null);
  const editingPropsRef = useRef(false);
  const editingTransformRef = useRef(false);
  const editingAudioRef = useRef(false);
  const captionTextareaFocusedRef = useRef(false);

  const selectedGfx =
    selectedClip?.clip.payload.kind === "gfx" ? selectedClip.clip.payload.gfx : null;
  const selectedCaption =
    selectedClip?.clip.payload.kind === "caption"
      ? selectedClip.clip.payload.caption
      : null;

  const captionWordsSig = selectedCaption
    ? selectedCaption.words.map((w) => `${w.text}:${w.start_sec}:${w.end_sec}`).join("|")
    : "";

  // Clear pending debounce timers on unmount
  useEffect(() => {
    return () => {
      if (propDebounceRef.current) clearTimeout(propDebounceRef.current);
      if (captionColorDebounceRef.current) clearTimeout(captionColorDebounceRef.current);
    };
  }, []);

  // Avoid mid-keystroke/mid-drag resets while syncing external agent edits or Cmd+Z
  useEffect(() => {
    const cid = selectedClip?.clip.id ?? null;
    if (cid !== lastPropsClipIdRef.current) {
      if (propDebounceRef.current) {
        clearTimeout(propDebounceRef.current);
        propDebounceRef.current = null;
      }
      if (captionColorDebounceRef.current) {
        clearTimeout(captionColorDebounceRef.current);
        captionColorDebounceRef.current = null;
      }
      lastPropsClipIdRef.current = cid;
      editingPropsRef.current = false;
      editingTransformRef.current = false;
      editingAudioRef.current = false;
      captionTextareaFocusedRef.current = false;
      setLocalProps(selectedGfx?.props ?? {});
      setTransformDraft(selectedClip?.clip.transform ?? null);
      setAudioDraft(selectedClip?.clip.audio ?? null);
      if (selectedCaption) {
        setCaptionTextDraft(
          selectedCaption.words.map((w) => w.text).join(" ") ||
            selectedClip?.clip.label ||
            "",
        );
        setCaptionColorDraft(
          /^#[0-9a-fA-F]{6}$/.test(selectedCaption.highlight_color)
            ? selectedCaption.highlight_color
            : "#F7B733",
        );
      } else {
        setCaptionTextDraft("");
        setCaptionColorDraft("#F7B733");
      }
      return;
    }
    if (!editingPropsRef.current && selectedGfx) {
      setLocalProps(selectedGfx.props ?? {});
    }
    if (!editingTransformRef.current && selectedClip) {
      setTransformDraft(selectedClip.clip.transform);
    }
    if (!editingAudioRef.current && selectedClip) {
      setAudioDraft(selectedClip.clip.audio);
    }
    if (!captionTextareaFocusedRef.current && selectedCaption) {
      setCaptionTextDraft(
        selectedCaption.words.map((w) => w.text).join(" ") ||
          selectedClip?.clip.label ||
          "",
      );
    }
    if (!captionColorDebounceRef.current && selectedCaption) {
      setCaptionColorDraft(
        /^#[0-9a-fA-F]{6}$/.test(selectedCaption.highlight_color)
          ? selectedCaption.highlight_color
          : "#F7B733",
      );
    }
  }, [
    selectedClip?.clip.id,
    selectedClip?.clip.label,
    selectedClip?.clip.transform,
    selectedClip?.clip.audio,
    selectedGfx,
    selectedCaption,
    captionWordsSig,
  ]);

  useEffect(() => {
    if (selectedCaption) {
      setCaptionPreset(selectedCaption.preset);
    } else if (activeCaptionPreset) {
      setCaptionPreset(activeCaptionPreset);
    }
  }, [activeCaptionPreset, selectedCaption]);

  const matchedAsset = selectedGfx?.asset_id
    ? savedAssets.find((a) => a.id === selectedGfx.asset_id)
    : undefined;
  const gfxFields = selectedGfx
    ? extractPropFields(matchedAsset?.props_schema ?? {}, localProps)
    : [];

  const gfxLintIssues = useMemo(() => {
    if (!selectedGfx?.inline_html) return [];
    // Omit the advisory brand-token hint on bespoke inline scenes so custom-palette
    // prompts don't show a false-positive warning in the Clip Inspector.
    return lintGfxHtml(selectedGfx.inline_html).filter(
      (issue) => issue.code !== "missing-brand-tokens",
    );
  }, [selectedGfx?.inline_html]);

  const handlePropChange = (key: string, nextVal: unknown) => {
    if (!selectedClip || !selectedGfx) return;
    editingPropsRef.current = true;
    const updated = { ...localProps, [key]: nextVal };
    setLocalProps(updated);
    if (propDebounceRef.current) clearTimeout(propDebounceRef.current);
    const tid = selectedClip.trackId;
    const cid = selectedClip.clip.id;
    propDebounceRef.current = setTimeout(() => {
      editingPropsRef.current = false;
      onUpdateGfxProps(tid, cid, updated);
    }, 240);
  };

  const commitCaptionText = () => {
    if (!selectedClip || !selectedCaption || !onUpdateCaptionClip) return;
    const tokens = captionTextDraft
      .trim()
      .split(/\s+/)
      .filter(Boolean);
    if (tokens.length === 0) return;
    const start = selectedClip.clip.start_sec;
    const dur = Math.max(0.25, selectedClip.clip.duration_sec);
    const step = dur / tokens.length;
    const words = tokens.map((text, idx) => ({
      text,
      start_sec: Number((start + idx * step).toFixed(3)),
      end_sec: Number((start + (idx + 1) * step).toFixed(3)),
    }));
    onUpdateCaptionClip(selectedClip.trackId, selectedClip.clip.id, {
      ...selectedCaption,
      words,
    });
  };

  const handleCaptionColorChange = (nextColor: string) => {
    if (!selectedClip || !selectedCaption || !onUpdateCaptionClip) return;
    setCaptionColorDraft(nextColor);
    if (captionColorDebounceRef.current) {
      clearTimeout(captionColorDebounceRef.current);
    }
    const tid = selectedClip.trackId;
    const cid = selectedClip.clip.id;
    const captionSnapshot = selectedCaption;
    captionColorDebounceRef.current = setTimeout(() => {
      captionColorDebounceRef.current = null;
      onUpdateCaptionClip(tid, cid, {
        ...captionSnapshot,
        highlight_color: nextColor,
      });
    }, 180);
  };

  const commitTransform = (next: Transform2D) => {
    if (!selectedClip || !onUpdateClipTransform) return;
    editingTransformRef.current = false;
    setTransformDraft(next);
    onUpdateClipTransform(selectedClip.trackId, selectedClip.clip.id, next);
  };

  const commitAudio = (next: AudioSettings) => {
    if (!selectedClip || !onUpdateClipAudio) return;
    editingAudioRef.current = false;
    setAudioDraft(next);
    onUpdateClipAudio(selectedClip.trackId, selectedClip.clip.id, next);
  };

  const effectiveTransform = transformDraft ?? selectedClip?.clip.transform ?? null;
  const effectiveAudio = audioDraft ?? selectedClip?.clip.audio ?? null;

  const showTransformControls =
    selectedClip !== null &&
    effectiveTransform !== null &&
    onUpdateClipTransform !== undefined &&
    (selectedClip.clip.payload.kind === "media" ||
      selectedClip.clip.payload.kind === "gfx" ||
      selectedClip.clip.payload.kind === "caption") &&
    selectedClip.trackId !== "a1" &&
    selectedClip.trackId !== "a2";

  const showAudioControls =
    selectedClip !== null &&
    effectiveAudio !== null &&
    onUpdateClipAudio !== undefined &&
    selectedClip.clip.payload.kind === "media";

  const hasClipControls = Boolean(
    selectedClip &&
      (selectedGfx ||
        (selectedCaption && onUpdateCaptionClip) ||
        showTransformControls ||
        showAudioControls),
  );

  const prevSelectedClipIdRef = useRef<string | null>(null);
  useEffect(() => {
    const cid = selectedClip?.clip.id ?? null;
    if (cid !== prevSelectedClipIdRef.current) {
      prevSelectedClipIdRef.current = cid;
      if (cid && hasClipControls) {
        setInspectorTab("clip");
      } else if (!cid && inspectorTab === "clip") {
        setInspectorTab("color");
      }
    }
  }, [selectedClip?.clip.id, hasClipControls, inspectorTab]);

  if (selectedClips && selectedClips.length > 1 && onApplyBatchCommand) {
    return (
      <BatchInspector
        selection={selectedClips}
        onApplyBatch={onApplyBatchCommand}
        onClearSelection={onClearSelection ?? (() => {})}
      />
    );
  }

  const activeTab = inspectorTab === "clip" && !hasClipControls ? "color" : inspectorTab;

  return (
    <aside className="flex h-full flex-col gap-3 overflow-y-auto rounded-2xl border border-areia/10 bg-carbono/70 p-3 text-xs">
      {/* Pro Segmented Inspector Tabs */}
      <div className="flex items-center justify-between gap-2 border-b border-areia/10 pb-2">
        <div
          role="tablist"
          aria-label="Inspector sections"
          className="flex items-center gap-0.5 rounded-full border border-areia/10 bg-void/70 p-0.5 text-[10px]"
        >
          <button
            type="button"
            role="tab"
            aria-selected={activeTab === "color"}
            onClick={() => setInspectorTab("color")}
            className={`rounded-full px-2.5 py-0.5 font-medium transition-colors motion-reduce:transition-none ${
              activeTab === "color"
                ? "bg-laranja-criacao/25 text-areia"
                : "text-areia/55 hover:text-areia"
            }`}
          >
            Color
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={activeTab === "captions"}
            onClick={() => setInspectorTab("captions")}
            className={`rounded-full px-2.5 py-0.5 font-medium transition-colors motion-reduce:transition-none ${
              activeTab === "captions"
                ? "bg-laranja-criacao/25 text-areia"
                : "text-areia/55 hover:text-areia"
            }`}
          >
            Captions
          </button>
          {hasClipControls && (
            <button
              type="button"
              role="tab"
              aria-selected={activeTab === "clip"}
              onClick={() => setInspectorTab("clip")}
              className={`flex items-center gap-1 rounded-full px-2.5 py-0.5 font-medium transition-colors motion-reduce:transition-none ${
                activeTab === "clip"
                  ? "bg-amarelo-ideias/20 text-amarelo-ideias"
                  : "text-areia/65 hover:text-areia"
              }`}
            >
              <span className="h-1.5 w-1.5 rounded-full bg-amarelo-ideias" />
              <span>Clip</span>
            </button>
          )}
        </div>
        <span className="truncate text-[10px] font-medium text-areia/45">
          {activeTab === "color"
            ? gradeTargetLabel
            : activeTab === "clip" && selectedClip
              ? selectedClip.clip.label
              : "Word-sync"}
        </span>
      </div>

      {activeTab === "clip" && (
        <>
      {/* Contextual Selected GFX Prop Editor */}
      {selectedClip && selectedGfx && (
        <div className="space-y-2.5 rounded-xl border border-track-g1/35 bg-void/70 p-2.5">
          <div className="flex items-center justify-between gap-1">
            <span className="truncate text-[10px] font-semibold uppercase tracking-[0.14em] text-track-g1">
              GFX · {selectedClip.clip.label}
            </span>
            <button
              type="button"
              disabled={savingAsset}
              onClick={async () => {
                setSavingAsset(true);
                try {
                  await onSaveClipAsAsset(selectedClip.clip);
                } finally {
                  setSavingAsset(false);
                }
              }}
              className="flex shrink-0 items-center gap-1 rounded-full border border-areia/14 px-2 py-0.5 text-[10px] text-areia/85 hover:border-amarelo-ideias/50 hover:bg-accent disabled:opacity-50"
              title="Save this component to your Reusable Asset Library"
            >
              <BookmarkPlus className="h-3 w-3 text-amarelo-ideias" />
              <span>{savingAsset ? "Saving..." : "Save"}</span>
            </button>
          </div>

          {gfxLintIssues.length > 0 && (
            <div className="flex items-start gap-1.5 rounded-lg border border-amarelo-ideias/35 bg-amarelo-ideias/10 px-2 py-1.5 text-[10px] text-areia/85">
              <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0 text-amarelo-ideias" />
              <span>{gfxLintIssues[0].message}</span>
            </div>
          )}

          {gfxFields.length === 0 ? (
            <div className="text-[10px] text-areia/45">No editable props</div>
          ) : (
            <div className="space-y-2">
              {gfxFields.map((field) =>
                field.kind === "boolean" ? (
                  <label
                    key={field.key}
                    className="flex items-center justify-between rounded-lg border border-areia/12 bg-carbono px-2.5 py-1.5"
                  >
                    <span className="text-[10px] uppercase tracking-[0.1em] text-areia/65">
                      {field.label}
                    </span>
                    <input
                      type="checkbox"
                      checked={Boolean(field.value)}
                      onChange={(e) => handlePropChange(field.key, e.target.checked)}
                      className="h-3.5 w-3.5 accent-laranja-criacao"
                    />
                  </label>
                ) : (
                  <label key={field.key} className="block space-y-1">
                    <span className="block text-[10px] uppercase tracking-[0.1em] text-areia/55">
                      {field.label}
                    </span>
                    <input
                      type={
                        field.kind === "number"
                          ? "number"
                          : field.kind === "color"
                            ? "color"
                            : "text"
                      }
                      value={String(field.value)}
                      onChange={(e) => {
                        const nextVal =
                          field.kind === "number"
                            ? Number(e.target.value)
                            : e.target.value;
                        handlePropChange(field.key, nextVal);
                      }}
                      className="w-full rounded-lg border border-areia/12 bg-carbono px-2.5 py-1 text-[11px] text-areia outline-none focus:border-laranja-criacao"
                    />
                  </label>
                ),
              )}
            </div>
          )}
        </div>
      )}

      {/* Contextual Caption Text & Highlight Editor */}
      {selectedClip && selectedCaption && onUpdateCaptionClip && (
        <div className="space-y-2 rounded-xl border border-track-t1/35 bg-void/70 p-2.5">
          <div className="flex items-center justify-between">
            <span className="text-[10px] font-semibold uppercase tracking-[0.14em] text-track-t1">
              Caption Clip
            </span>
            <input
              type="color"
              title="Highlight color"
              value={captionColorDraft}
              onChange={(e) => handleCaptionColorChange(e.target.value)}
              className="h-4 w-5 cursor-pointer rounded border-0 bg-transparent p-0"
            />
          </div>
          <textarea
            rows={2}
            value={captionTextDraft}
            onFocus={() => {
              captionTextareaFocusedRef.current = true;
            }}
            onChange={(e) => setCaptionTextDraft(e.target.value)}
            onBlur={() => {
              captionTextareaFocusedRef.current = false;
              commitCaptionText();
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                commitCaptionText();
              }
            }}
            placeholder="Edit caption words..."
            className="w-full resize-none rounded-lg border border-areia/12 bg-carbono px-2 py-1 text-[11px] text-areia outline-none focus:border-laranja-criacao"
          />
        </div>
      )}

      {/* Contextual Transform2D Controls */}
      {showTransformControls && selectedClip && effectiveTransform && (
        <div className="space-y-2 rounded-xl border border-areia/12 bg-void/70 p-2.5">
          <div className="flex items-center justify-between text-[10px] font-semibold uppercase tracking-[0.14em] text-areia/65">
            <span className="truncate">Transform · {selectedClip.clip.label}</span>
            <button
              type="button"
              onClick={() =>
                commitTransform({
                  ...effectiveTransform,
                  x: 0,
                  y: 0,
                  scale: 1,
                  rotation_deg: 0,
                  opacity: 1,
                })
              }
              className="text-[10px] font-normal normal-case text-amarelo-ideias/80 hover:underline"
            >
              Reset
            </button>
          </div>
          <div className="grid grid-cols-2 gap-1.5 text-[10px]">
            {(
              [
                ["x", "Pos X", -0.5, 0.5, 0.02],
                ["y", "Pos Y", -0.5, 0.5, 0.02],
                ["scale", "Scale", 0.2, 2.5, 0.02],
                ["rotation_deg", "Rot °", -180, 180, 1],
                ["opacity", "Opacity", 0, 1, 0.02],
              ] as const
            ).map(([field, label, min, max, step]) => (
              <label key={field} className="flex flex-col gap-0.5">
                <div className="flex justify-between text-areia/55">
                  <span>{label}</span>
                  <span className="tabular-nums text-areia/80">
                    {field === "rotation_deg"
                      ? `${Math.round(effectiveTransform[field])}°`
                      : effectiveTransform[field].toFixed(2)}
                  </span>
                </div>
                <input
                  type="range"
                  min={min}
                  max={max}
                  step={step}
                  value={effectiveTransform[field]}
                  onChange={(e) => {
                    const val = parseFloat(e.target.value);
                    if (!Number.isFinite(val)) return;
                    editingTransformRef.current = true;
                    setTransformDraft({
                      ...effectiveTransform,
                      [field]: val,
                    });
                  }}
                  onPointerUp={(e) => {
                    const val = parseFloat(e.currentTarget.value);
                    if (!Number.isFinite(val)) return;
                    commitTransform({
                      ...effectiveTransform,
                      [field]: val,
                    });
                  }}
                  onKeyUp={(e) => {
                    if (!SLIDER_COMMIT_KEYS.has(e.key)) return;
                    const val = parseFloat(e.currentTarget.value);
                    if (!Number.isFinite(val)) return;
                    commitTransform({
                      ...effectiveTransform,
                      [field]: val,
                    });
                  }}
                  onBlur={() => {
                    if (editingTransformRef.current) {
                      commitTransform(effectiveTransform);
                    }
                  }}
                  className="h-1 w-full cursor-pointer accent-laranja-criacao"
                />
              </label>
            ))}
          </div>
        </div>
      )}

      {/* Contextual AudioSettings Controls */}
      {showAudioControls && selectedClip && effectiveAudio && (
        <div className="space-y-2 rounded-xl border border-track-a1/30 bg-void/70 p-2.5">
          <div className="flex items-center justify-between text-[10px] font-semibold uppercase tracking-[0.14em] text-track-a1">
            <span>Audio Mix</span>
            <label className="flex items-center gap-1 text-[10px] font-normal normal-case text-areia/80">
              <input
                type="checkbox"
                checked={effectiveAudio.ducking_enabled}
                onChange={(e) =>
                  commitAudio({
                    ...effectiveAudio,
                    ducking_enabled: e.target.checked,
                  })
                }
                className="h-3 w-3 accent-laranja-criacao"
              />
              <span>Duck</span>
            </label>
          </div>
          <div className="space-y-1.5 text-[10px]">
            {(
              [
                ["gain_db", "Gain dB", -48, 12, 0.5],
                ["duck_target_db", "Duck dB", -36, -6, 1],
                ["fade_in_sec", "Fade In", 0, 3, 0.05],
                ["fade_out_sec", "Fade Out", 0, 3, 0.05],
              ] as const
            ).map(([field, label, min, max, step]) => (
              <label key={field} className="flex items-center gap-2">
                <span className="w-14 text-areia/60">{label}</span>
                <input
                  type="range"
                  min={min}
                  max={max}
                  step={step}
                  value={Math.max(min, Math.min(max, effectiveAudio[field]))}
                  onChange={(e) => {
                    const val = parseFloat(e.target.value);
                    if (!Number.isFinite(val)) return;
                    editingAudioRef.current = true;
                    setAudioDraft({
                      ...effectiveAudio,
                      [field]: val,
                    });
                  }}
                  onPointerUp={(e) => {
                    const val = parseFloat(e.currentTarget.value);
                    if (!Number.isFinite(val)) return;
                    commitAudio({
                      ...effectiveAudio,
                      [field]: val,
                    });
                  }}
                  onKeyUp={(e) => {
                    if (!SLIDER_COMMIT_KEYS.has(e.key)) return;
                    const val = parseFloat(e.currentTarget.value);
                    if (!Number.isFinite(val)) return;
                    commitAudio({
                      ...effectiveAudio,
                      [field]: val,
                    });
                  }}
                  onBlur={() => {
                    if (editingAudioRef.current) {
                      commitAudio(effectiveAudio);
                    }
                  }}
                  className="h-1 flex-1 cursor-pointer accent-amarelo-ideias"
                />
                <span className="w-10 text-right tabular-nums text-areia/80">
                  {effectiveAudio[field].toFixed(1)}
                </span>
              </label>
            ))}
          </div>
        </div>
      )}

        </>
      )}

      {/* Interactive 3-Way Color Wheels + Sliders */}
      <div
        data-cursor-anchor="color.inspector"
        className={activeTab === "color" ? "space-y-3" : "hidden"}
      >
        <div className="flex items-center justify-between gap-1.5 text-[10px] font-semibold uppercase tracking-[0.14em] text-areia/60">
          <span>Master Wheels</span>
          <span
            className={`truncate rounded-full px-2 py-0.5 text-[10px] font-semibold normal-case tracking-normal ${
              gradeTargetLabel.startsWith("FX")
                ? "border border-track-fx/45 bg-track-fx/15 text-areia"
                : "text-amarelo-ideias/80"
            }`}
            title={`Color controls currently target ${gradeTargetLabel}`}
          >
            {gradeTargetLabel}
          </span>
        </div>

        <div className="grid grid-cols-3 gap-1.5 text-center">
          <ColorWheelControl
            label="LIFT"
            value={grade.lift}
            onChange={(lift) => onPreviewGrade({ ...grade, lift })}
            onCommit={(lift) => onCommitGrade({ ...grade, lift })}
          />
          <ColorWheelControl
            label="GAMMA"
            value={grade.gamma}
            onChange={(gamma) => onPreviewGrade({ ...grade, gamma })}
            onCommit={(gamma) => onCommitGrade({ ...grade, gamma })}
          />
          <ColorWheelControl
            label="GAIN"
            value={grade.gain}
            onChange={(gain) => onPreviewGrade({ ...grade, gain })}
            onCommit={(gain) => onCommitGrade({ ...grade, gain })}
          />
        </div>

        <div className="space-y-2 pt-1">
          {(
            [
              ["temp", "Temp", -1, 1, 0.02],
              ["tint", "Tint", -1, 1, 0.02],
              ["contrast", "Contrast", 0.5, 1.6, 0.02],
              ["saturation", "Saturation", 0, 2, 0.02],
              ["exposure", "Exposure", -1, 1, 0.02],
            ] as const
          ).map(([field, label, min, max, step]) => (
            <label key={field} className="flex items-center gap-2 text-[11px]">
              <span className="w-16 text-areia/65">{label}</span>
              <input
                type="range"
                min={min}
                max={max}
                step={step}
                value={grade[field]}
                onChange={(e) =>
                  onPreviewGrade({
                    ...grade,
                    [field]: parseFloat(e.target.value),
                  })
                }
                onPointerUp={(e) =>
                  onCommitGrade({
                    ...grade,
                    [field]: parseFloat(e.currentTarget.value),
                  })
                }
                onKeyUp={(e) => {
                  if (!SLIDER_COMMIT_KEYS.has(e.key)) return;
                  onCommitGrade({
                    ...grade,
                    [field]: parseFloat(e.currentTarget.value),
                  });
                }}
                className="h-1 flex-1 cursor-pointer accent-laranja-criacao"
              />
              <span className="w-9 text-right tabular-nums text-[10px] text-areia/75">
                {grade[field].toFixed(2)}
              </span>
            </label>
          ))}

          {hasBrandLut && (
            <label className="flex items-center gap-2 text-[11px]">
              <span className="w-16 text-amarelo-ideias/85">LUT Mix</span>
              <input
                type="range"
                min={0}
                max={1}
                step={0.02}
                value={grade.lut_intensity}
                onChange={(e) =>
                  onPreviewGrade({
                    ...grade,
                    lut_intensity: parseFloat(e.target.value),
                  })
                }
                onPointerUp={(e) =>
                  onCommitGrade({
                    ...grade,
                    lut_intensity: parseFloat(e.currentTarget.value),
                  })
                }
                onKeyUp={(e) => {
                  if (!SLIDER_COMMIT_KEYS.has(e.key)) return;
                  onCommitGrade({
                    ...grade,
                    lut_intensity: parseFloat(e.currentTarget.value),
                  });
                }}
                className="h-1 flex-1 cursor-pointer accent-amarelo-ideias"
              />
              <span className="w-9 text-right tabular-nums text-[10px] text-amarelo-ideias">
                {Math.round(grade.lut_intensity * 100)}%
              </span>
            </label>
          )}
        </div>
      </div>

      {/* Captions */}
      <div
        data-cursor-anchor="captions.inspector"
        className={activeTab === "captions" ? "space-y-3" : "hidden"}
      >
        <div className="flex items-center justify-between text-[10px] font-semibold uppercase tracking-[0.14em] text-areia/60">
          <span>Caption Style</span>
          <span className="text-areia/45">word-by-word</span>
        </div>
        <div className="grid grid-cols-2 gap-1.5">
          {CAPTION_PRESETS.map((preset) => (
            <button
              key={preset.id}
              type="button"
              onClick={() => {
                setCaptionPreset(preset.id);
                onUpdateCaptionPreset?.(preset.id);
              }}
              className={`rounded-xl border px-2.5 py-1.5 text-left text-[11px] transition-colors motion-reduce:transition-none ${
                captionPreset === preset.id
                  ? "border-amarelo-ideias/60 bg-accent text-areia"
                  : "border-areia/10 bg-void/60 text-areia/70 hover:bg-accent"
              }`}
            >
              {preset.label}
            </button>
          ))}
        </div>
      </div>

      {/* Swarm Telemetry */}
      <div className="mt-auto space-y-2 border-t border-areia/10 pt-3">
        <div className="flex items-center justify-between text-[10px] font-semibold uppercase tracking-[0.14em] text-areia/60">
          <span>Swarm Roles</span>
          <span className="tabular-nums text-amarelo-ideias">6 roles</span>
        </div>
        <div className="grid grid-cols-2 gap-x-3 gap-y-1.5 text-[10px]">
          {[
            ["caps", "bg-track-t1"],
            ["b_roll", "bg-track-v2"],
            ["grade", "bg-track-fx"],
            ["razor", "bg-track-v1"],
            ["gfx", "bg-track-g1"],
            ["mix", "bg-track-a1"],
          ].map(([role, colorClass]) => {
            const count = swarmRoleCounts[role] ?? 0;
            const widthPct = count === 0 ? 0 : Math.min(100, 22 + count * 18);
            return (
              <div key={role} className="flex items-center gap-1.5">
                <span className="w-9 text-areia/70">{role}</span>
                <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-grafite">
                  <div
                    className={`h-full rounded-full ${colorClass}`}
                    style={{ width: `${widthPct}%` }}
                  />
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </aside>
  );
}
