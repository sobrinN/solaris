import { useRef, useState } from "react";
import {
  Captions,
  Film,
  Image as ImageIcon,
  Loader2,
  Mic,
  Music,
  Plus,
  Sparkles,
  Trash2,
  Upload,
} from "lucide-react";
import type { CaptionPreset, MediaAsset } from "@solaris/engine-client";
import { formatCompactDuration } from "../../lib/timelineFormat";

export interface MediaBinPanelProps {
  mediaPool: MediaAsset[];
  selectedMediaId: string | null;
  importing: boolean;
  transcribingId: string | null;
  generatingCaptions: boolean;
  onImportFiles: (files: File[]) => Promise<void>;
  onSelectMedia: (mediaId: string) => void;
  onPreviewInSource: (mediaId: string) => void;
  onAddToTimeline: (media: MediaAsset) => Promise<void>;
  onUpdateRole: (mediaId: string, roleHint: string) => Promise<void>;
  onTranscribe: (mediaId: string) => Promise<void>;
  onAutoCaptions: (media: MediaAsset) => Promise<void>;
  onDeleteMedia: (mediaId: string) => Promise<void>;
  captionPreset?: CaptionPreset;
}

const ROLE_OPTIONS: Array<{ value: string; label: string }> = [
  { value: "a_roll", label: "A-Roll" },
  { value: "b_roll", label: "B-Roll" },
  { value: "voice", label: "Voice" },
  { value: "music", label: "Music" },
  { value: "sfx", label: "SFX" },
];

function KindIcon({ kind, roleHint }: { kind: MediaAsset["kind"]; roleHint: string }) {
  if (kind === "image") return <ImageIcon className="h-3.5 w-3.5 text-track-v2" />;
  if (kind === "audio") {
    return roleHint === "voice" || roleHint === "voiceover" ? (
      <Mic className="h-3.5 w-3.5 text-track-a1" />
    ) : (
      <Music className="h-3.5 w-3.5 text-track-a2" />
    );
  }
  return <Film className="h-3.5 w-3.5 text-track-v1" />;
}

export function MediaBinPanel({
  mediaPool,
  selectedMediaId,
  importing,
  transcribingId,
  generatingCaptions,
  onImportFiles,
  onSelectMedia,
  onPreviewInSource,
  onAddToTimeline,
  onUpdateRole,
  onTranscribe,
  onAutoCaptions,
  onDeleteMedia,
}: MediaBinPanelProps) {
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const [dragActive, setDragActive] = useState(false);

  const handleFiles = (list: FileList | null) => {
    if (!list || list.length === 0) return;
    void onImportFiles(Array.from(list));
  };

  return (
    <div
      className="flex min-h-0 flex-1 flex-col"
      onDragOver={(e) => {
        if (e.dataTransfer.types.includes("Files")) {
          e.preventDefault();
          setDragActive(true);
        }
      }}
      onDragLeave={() => setDragActive(false)}
      onDrop={(e) => {
        setDragActive(false);
        if (e.dataTransfer.files.length > 0) {
          e.preventDefault();
          handleFiles(e.dataTransfer.files);
        }
      }}
    >
      <input
        ref={fileInputRef}
        type="file"
        multiple
        accept="video/*,image/*,audio/*,.pdf,.md,.txt"
        className="hidden"
        onChange={(e) => {
          handleFiles(e.target.files);
          e.currentTarget.value = "";
        }}
      />

      <div className="flex items-center justify-between gap-2 border-b border-areia/[0.08] px-3 py-2">
        <div className="text-[10px] font-semibold uppercase tracking-[0.14em] text-areia/60">
          Project Media ({mediaPool.length})
        </div>
        <button
          type="button"
          disabled={importing}
          onClick={() => fileInputRef.current?.click()}
          className="inline-flex h-7 items-center gap-1.5 rounded-lg border border-laranja-criacao/40 bg-laranja-criacao/15 px-2.5 text-[10px] font-semibold text-areia transition-colors hover:bg-laranja-criacao/25 disabled:opacity-45 motion-reduce:transition-none"
        >
          {importing ? (
            <Loader2 className="h-3 w-3 animate-spin motion-reduce:animate-none" />
          ) : (
            <Upload className="h-3 w-3 text-amarelo-ideias" />
          )}
          <span>Import Media</span>
        </button>
      </div>

      <div
        className={`flex-1 space-y-2 overflow-y-auto p-3 transition-colors motion-reduce:transition-none ${
          dragActive ? "bg-laranja-criacao/10" : ""
        }`}
      >
        {mediaPool.length === 0 ? (
          <button
            type="button"
            onClick={() => fileInputRef.current?.click()}
            className="flex w-full flex-col items-center justify-center gap-2 rounded-xl border border-dashed border-areia/20 bg-void/45 px-4 py-8 text-center transition-colors hover:border-laranja-criacao/45 hover:bg-accent motion-reduce:transition-none"
          >
            <Upload className="h-5 w-5 text-amarelo-ideias" />
            <p className="text-xs font-medium text-areia">Drop video, audio, or images here</p>
            <p className="text-[10px] text-areia/50">
              Drag media onto any timeline track or transcribe with Whisper for synced captions
            </p>
          </button>
        ) : (
          mediaPool.map((media) => {
            const isSelected = selectedMediaId === media.id;
            const intel = media.intelligence;
            const hasSpeechCapable =
              media.kind === "audio" || (media.kind === "video" && intel.has_audio !== false);
            const isTranscribing = transcribingId === media.id;
            const dims =
              intel.width && intel.height ? `${intel.width}×${intel.height}` : null;
            const fpsLabel = intel.fps ? `${Math.round(intel.fps)}fps` : null;
            const durLabel =
              intel.duration_sec > 0 ? formatCompactDuration(intel.duration_sec) : null;

            return (
              <div
                key={media.id}
                draggable
                onDragStart={(e) => {
                  e.dataTransfer.effectAllowed = "copy";
                  e.dataTransfer.setData("application/x-solaris-media-id", media.id);
                }}
                onClick={() => onSelectMedia(media.id)}
                className={`group rounded-xl border p-2.5 transition-colors motion-reduce:transition-none ${
                  isSelected
                    ? "border-amarelo-ideias/55 bg-carbono/95"
                    : "border-areia/[0.09] bg-void/65 hover:border-areia/20 hover:bg-carbono/70"
                }`}
              >
                <div className="flex items-start justify-between gap-2">
                  <div className="flex min-w-0 items-center gap-2">
                    <span className="grid h-7 w-7 shrink-0 place-items-center rounded-lg border border-areia/10 bg-carbono">
                      <KindIcon kind={media.kind} roleHint={media.role_hint} />
                    </span>
                    <div className="min-w-0">
                      <div className="truncate text-[11px] font-semibold text-areia">
                        {media.name}
                      </div>
                      <div className="flex flex-wrap items-center gap-1.5 text-[10px] tabular-nums text-areia/55">
                        {dims && <span>{dims}</span>}
                        {fpsLabel && <span>· {fpsLabel}</span>}
                        {durLabel && <span>· {durLabel}</span>}
                        {!intel.analyzed && (
                          <span className="text-amarelo-ideias">Analyzing…</span>
                        )}
                        {intel.is_log_footage && (
                          <span className="rounded bg-amarelo-ideias/15 px-1 text-[9px] font-semibold text-amarelo-ideias">
                            LOG
                          </span>
                        )}
                      </div>
                    </div>
                  </div>

                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation();
                      void onDeleteMedia(media.id);
                    }}
                    title="Remove media from project"
                    className="rounded-md p-1 text-areia/40 transition-colors hover:bg-vermelho-foco/20 hover:text-areia motion-reduce:transition-none"
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </button>
                </div>

                <div className="mt-2 flex flex-wrap items-center gap-1.5">
                  <select
                    aria-label={`Role for ${media.name}`}
                    value={media.role_hint}
                    onClick={(e) => e.stopPropagation()}
                    onChange={(e) => void onUpdateRole(media.id, e.target.value)}
                    className="h-6 rounded-md border border-areia/12 bg-carbono px-1.5 text-[10px] text-areia/85 focus:border-laranja-criacao focus:outline-none"
                  >
                    {ROLE_OPTIONS.map((opt) => (
                      <option key={opt.value} value={opt.value}>
                        {opt.label}
                      </option>
                    ))}
                  </select>

                  {media.kind === "video" && (
                    <button
                      type="button"
                      onClick={(e) => {
                        e.stopPropagation();
                        onPreviewInSource(media.id);
                      }}
                      className="h-6 rounded-md border border-areia/12 bg-void/70 px-2 text-[10px] font-medium text-areia/80 hover:bg-accent hover:text-areia"
                    >
                      Source
                    </button>
                  )}

                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation();
                      void onAddToTimeline(media);
                    }}
                    className="inline-flex h-6 items-center gap-1 rounded-md border border-areia/14 bg-carbono px-2 text-[10px] font-medium text-areia hover:border-laranja-criacao/50 hover:bg-accent"
                  >
                    <Plus className="h-3 w-3 text-amarelo-ideias" />
                    <span>Timeline</span>
                  </button>

                  {hasSpeechCapable && (
                    <>
                      <button
                        type="button"
                        disabled={isTranscribing}
                        onClick={(e) => {
                          e.stopPropagation();
                          void onTranscribe(media.id);
                        }}
                        title="Run Whisper / STT transcription"
                        className="inline-flex h-6 items-center gap-1 rounded-md border border-areia/12 bg-void/70 px-2 text-[10px] font-medium text-areia/80 hover:bg-accent hover:text-areia disabled:opacity-45"
                      >
                        {isTranscribing ? (
                          <Loader2 className="h-3 w-3 animate-spin motion-reduce:animate-none" />
                        ) : (
                          <Sparkles className="h-3 w-3 text-amarelo-ideias" />
                        )}
                        <span>
                          {intel.transcript.length > 0
                            ? `${intel.transcript.length}w`
                            : "Transcribe"}
                        </span>
                      </button>

                      <button
                        type="button"
                        disabled={isTranscribing || generatingCaptions}
                        onClick={(e) => {
                          e.stopPropagation();
                          void onAutoCaptions(media);
                        }}
                        title="Transcribe if needed and place word-synced captions on T1"
                        className="inline-flex h-6 items-center gap-1 rounded-md border border-track-t1/40 bg-track-t1/15 px-2 text-[10px] font-medium text-areia hover:bg-track-t1/25 disabled:opacity-45"
                      >
                        <Captions className="h-3 w-3 text-amarelo-ideias" />
                        <span>Captions</span>
                      </button>
                    </>
                  )}
                </div>
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
