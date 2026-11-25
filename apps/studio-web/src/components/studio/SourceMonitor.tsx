import { useEffect, useRef, useState } from "react";
import { Music, Plus } from "lucide-react";
import type { MediaAsset } from "@solaris/engine-client";

interface SourceMonitorProps {
  media: MediaAsset | null;
  streamUrl: string | null;
  aspectBadge: string;
  canvasAspect?: number;
  reframeBoxWidthPct: number;
  /** Inserts the marked [IN, OUT] range of `media` at the timeline playhead. */
  onInsertRange: (media: MediaAsset, sourceIn: number, duration: number) => void;
}

/**
 * Raw clip preview with IN/OUT trim markers (the "Source" tab of the viewer).
 * Mount with `key={media.id}` so markers reset when the selected clip changes.
 */
export function SourceMonitor({
  media,
  streamUrl,
  aspectBadge,
  canvasAspect,
  reframeBoxWidthPct,
  onInsertRange,
}: SourceMonitorProps) {
  const [inSec, setInSec] = useState(0);
  const [outSec, setOutSec] = useState<number | null>(null);
  const [loadedDurationSec, setLoadedDurationSec] = useState<number | null>(null);
  const [loadedAspect, setLoadedAspect] = useState<number | null>(null);
  const mediaElRef = useRef<HTMLMediaElement | null>(null);

  useEffect(() => {
    return () => {
      const el = mediaElRef.current;
      if (el) {
        el.pause();
      }
    };
  }, []);

  const totalSec =
    loadedDurationSec !== null &&
    Number.isFinite(loadedDurationSec) &&
    loadedDurationSec > 0.2
      ? loadedDurationSec
      : media && media.intelligence.duration_sec > 0.2
        ? media.intelligence.duration_sec
        : 5;

  const safeInSec = Math.max(0, Math.min(inSec, totalSec));
  const safeOutSec =
    outSec !== null ? Math.max(safeInSec, Math.min(outSec, totalSec)) : null;

  const resolvedCanvasAspect =
    canvasAspect && Number.isFinite(canvasAspect) && canvasAspect > 0
      ? canvasAspect
      : aspectBadge === "9:16"
        ? 9 / 16
        : aspectBadge === "1:1"
          ? 1
          : 16 / 9;

  const intelWidth = media?.intelligence.width ?? null;
  const intelHeight = media?.intelligence.height ?? null;
  const srcAspect =
    loadedAspect ??
    (intelWidth && intelHeight && intelWidth > 0 && intelHeight > 0
      ? intelWidth / intelHeight
      : null);

  const showReframeOverlay =
    (media?.kind === "video" || media?.kind === "image") &&
    Boolean(streamUrl) &&
    srcAspect !== null &&
    Math.abs(srcAspect - resolvedCanvasAspect) > 0.08;

  const computedReframeWidthPct =
    srcAspect !== null && srcAspect > resolvedCanvasAspect
      ? Math.max(15, Math.min(96, (resolvedCanvasAspect / srcAspect) * 100))
      : reframeBoxWidthPct;

  const handleMarkIn = () => {
    const raw = mediaElRef.current?.currentTime ?? 0;
    const clamped = Number(Math.max(0, Math.min(totalSec, raw)).toFixed(2));
    setInSec(clamped);
    if (outSec !== null && clamped >= outSec) {
      setOutSec(null);
    }
  };

  const handleMarkOut = () => {
    const raw = mediaElRef.current?.currentTime ?? totalSec;
    const clamped = Number(Math.max(0, Math.min(totalSec, raw)).toFixed(2));
    if (clamped <= inSec) {
      setInSec(0);
    }
    setOutSec(clamped);
  };

  const handleInsert = () => {
    if (!media) return;
    const inPt = Math.max(0, Math.min(safeInSec, Math.max(0, totalSec - 0.25)));
    const outPt =
      safeOutSec !== null && safeOutSec > inPt + 0.15
        ? Math.min(safeOutSec, totalSec)
        : totalSec;
    onInsertRange(media, inPt, Math.max(0.25, outPt - inPt));
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      <div className="relative grid min-h-0 flex-1 place-items-center overflow-hidden rounded-xl border border-areia/12 bg-void">
        {media?.kind === "video" && streamUrl ? (
          <video
            ref={(el) => {
              mediaElRef.current = el;
            }}
            src={streamUrl}
            onLoadedMetadata={(e) => {
              const el = e.currentTarget;
              if (Number.isFinite(el.duration) && el.duration > 0) {
                setLoadedDurationSec(el.duration);
              }
              if (el.videoWidth > 0 && el.videoHeight > 0) {
                setLoadedAspect(el.videoWidth / el.videoHeight);
              }
            }}
            className="h-full w-full object-contain"
            controls
            muted
          />
        ) : media?.kind === "image" && streamUrl ? (
          <img
            src={streamUrl}
            alt={media.name}
            onLoad={(e) => {
              const el = e.currentTarget;
              if (el.naturalWidth > 0 && el.naturalHeight > 0) {
                setLoadedAspect(el.naturalWidth / el.naturalHeight);
              }
            }}
            className="h-full w-full object-contain"
          />
        ) : media?.kind === "audio" ? (
          <div className="flex w-full flex-col items-center gap-2 px-3 text-track-a1">
            <Music className="h-5 w-5" />
            <span className="max-w-full truncate text-[10px] text-areia/80">{media.name}</span>
            {streamUrl && (
              <audio
                ref={(el) => {
                  mediaElRef.current = el;
                }}
                src={streamUrl}
                onLoadedMetadata={(e) => {
                  const el = e.currentTarget;
                  if (Number.isFinite(el.duration) && el.duration > 0) {
                    setLoadedDurationSec(el.duration);
                  }
                }}
                controls
                className="h-8 w-full max-w-[260px]"
              />
            )}
          </div>
        ) : (
          <span className="px-4 text-center text-[11px] text-areia/45">
            {media
              ? "No preview available for this clip."
              : "Select a clip in Media to preview and trim it here."}
          </span>
        )}
        {showReframeOverlay && (
          <div
            className="pointer-events-none absolute inset-y-2 rounded border border-dashed border-amarelo-ideias/75"
            style={{ width: `${computedReframeWidthPct}%` }}
          >
            <span className="bg-amarelo-ideias px-1 text-[10px] font-semibold text-void">
              REFRAME {aspectBadge}
            </span>
          </div>
        )}
      </div>

      {media && media.kind !== "document" && (
        <div className="flex flex-wrap items-center justify-between gap-1.5 text-[10px]">
          {media.kind !== "image" && (
            <div className="flex items-center gap-1">
              <button
                type="button"
                onClick={handleMarkIn}
                className="rounded border border-areia/14 bg-void/70 px-1.5 py-0.5 text-areia/80 hover:border-amarelo-ideias/50"
                title="Mark Source IN at current source time"
              >
                IN <span className="tabular-nums text-amarelo-ideias">{safeInSec.toFixed(1)}s</span>
              </button>
              <button
                type="button"
                onClick={handleMarkOut}
                className="rounded border border-areia/14 bg-void/70 px-1.5 py-0.5 text-areia/80 hover:border-amarelo-ideias/50"
                title="Mark Source OUT at current source time"
              >
                OUT{" "}
                <span className="tabular-nums text-amarelo-ideias">
                  {(safeOutSec ?? totalSec).toFixed(1)}s
                </span>
              </button>
            </div>
          )}
          <button
            type="button"
            onClick={handleInsert}
            title="Insert the IN/OUT range at the timeline playhead"
            className="ml-auto flex items-center gap-1 rounded-full border border-laranja-criacao/50 bg-laranja-criacao/15 px-2.5 py-0.5 font-medium text-areia hover:bg-laranja-criacao/25"
          >
            <Plus className="h-2.5 w-2.5 text-amarelo-ideias" />
            <span>Insert Range</span>
          </button>
        </div>
      )}
    </div>
  );
}
