import { useEffect, useState } from "react";
import { CheckCircle2, Download, Film, FolderDown, Loader2, X, XCircle } from "lucide-react";
import type {
  ExportFormat,
  ExportJobStatus,
  ExportQuality,
  SolarisEngineClient,
} from "@solaris/engine-client";
import { isDesktopTauri, saveDesktopExportFile } from "../../lib/engineBridge";

interface ExportModalProps {
  open: boolean;
  projectId: string;
  projectName: string;
  client: SolarisEngineClient;
  activeJob: ExportJobStatus | null;
  onJobStarted: (job: ExportJobStatus) => void;
  onClose: () => void;
}

export function ExportModal({
  open,
  projectId,
  projectName,
  client,
  activeJob,
  onJobStarted,
  onClose,
}: ExportModalProps) {
  const [format, setFormat] = useState<ExportFormat>("mp4");
  const [quality, setQuality] = useState<ExportQuality>("standard");
  const [targetLufsInput, setTargetLufsInput] = useState("-14");
  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [desktopSaveMsg, setDesktopSaveMsg] = useState<string | null>(null);
  const [savingDesktop, setSavingDesktop] = useState(false);

  const desktopMode = isDesktopTauri();

  const isJobActive =
    activeJob !== null &&
    (activeJob.state === "queued" ||
      activeJob.state === "rendering_gfx" ||
      activeJob.state === "compositing");

  useEffect(() => {
    if (!open) return;
    setSubmitError(null);
    setDesktopSaveMsg(null);
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  if (!open) return null;

  const parseClampedLufs = (): number => {
    const parsed = parseFloat(targetLufsInput);
    if (!Number.isFinite(parsed)) return -14;
    return Math.max(-24, Math.min(-8, parsed));
  };

  const handleStart = async () => {
    if (submitting || isJobActive) return;
    const clampedLufs = parseClampedLufs();
    setTargetLufsInput(String(clampedLufs));
    setSubmitting(true);
    setSubmitError(null);
    setDesktopSaveMsg(null);
    try {
      const job = await client.startExport(projectId, {
        format,
        quality,
        target_lufs: clampedLufs,
        output_name: projectName,
      });
      onJobStarted(job);
    } catch (err) {
      setSubmitError(err instanceof Error ? err.message : "Failed to start export");
    } finally {
      setSubmitting(false);
    }
  };

  const handleDesktopSave = async () => {
    if (!activeJob?.output_path || savingDesktop) return;
    setSavingDesktop(true);
    setSubmitError(null);
    setDesktopSaveMsg(null);
    try {
      const outputExtMatch = activeJob.output_path.match(/\.([a-zA-Z0-9]{2,5})$/);
      const ext =
        outputExtMatch?.[1]?.toLowerCase() ??
        (format === "mov_pro_res4444" ? "mov" : format === "webm_alpha" ? "webm" : "mp4");
      const savedPath = await saveDesktopExportFile(
        activeJob.output_path,
        `${projectName || "solaris-export"}.${ext}`,
      );
      if (savedPath) {
        setDesktopSaveMsg(`Saved to ${savedPath}`);
      }
    } catch (err) {
      setSubmitError(
        err instanceof Error ? err.message : "Failed to save export to Downloads",
      );
    } finally {
      setSavingDesktop(false);
    }
  };

  const progressScale = Math.max(0, Math.min(1, activeJob?.progress ?? 0));

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="export-modal-title"
      className="fixed inset-0 z-50 grid place-items-center bg-void/75 p-4 backdrop-blur-sm"
    >
      <div className="w-full max-w-md rounded-2xl border border-areia/14 bg-carbono p-5 shadow-2xl">
        <div className="flex items-center justify-between border-b border-areia/10 pb-3">
          <div className="flex items-center gap-2">
            <Film className="h-4 w-4 text-laranja-criacao" />
            <h2 id="export-modal-title" className="text-sm font-semibold text-areia">
              Export Render
            </h2>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="grid h-8 w-8 place-items-center rounded-full border border-areia/12 text-areia/75 hover:text-areia"
            aria-label="Close export modal"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="mt-4 space-y-3.5 text-xs">
          {submitError && (
            <div className="rounded-xl border border-vermelho-foco/40 bg-vermelho-foco/15 p-2.5 text-[11px] text-areia">
              {submitError}
            </div>
          )}

          <div>
            <label className="mb-1.5 block text-[11px] uppercase tracking-[0.14em] text-areia/60">
              Format
            </label>
            <select
              value={format}
              onChange={(e) => setFormat(e.target.value as ExportFormat)}
              className="h-9 w-full rounded-xl border border-areia/12 bg-void/75 px-3 text-xs text-areia outline-none focus:border-laranja-criacao"
            >
              <option value="mp4" className="bg-carbono">
                MP4 · H.264 + AAC (Social / Delivery)
              </option>
              <option value="mov_pro_res4444" className="bg-carbono">
                MOV · ProRes 4444 Alpha (NLE Master)
              </option>
              <option value="webm_alpha" className="bg-carbono">
                WebM · VP9 Alpha (Web Overlay)
              </option>
            </select>
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="mb-1.5 block text-[11px] uppercase tracking-[0.14em] text-areia/60">
                Quality
              </label>
              <select
                value={quality}
                onChange={(e) => setQuality(e.target.value as ExportQuality)}
                className="h-9 w-full rounded-xl border border-areia/12 bg-void/75 px-3 text-xs text-areia outline-none focus:border-laranja-criacao"
              >
                <option value="draft" className="bg-carbono">Draft (Fast)</option>
                <option value="standard" className="bg-carbono">Standard</option>
                <option value="high" className="bg-carbono">High Master</option>
              </select>
            </div>

            <div>
              <label className="mb-1.5 block text-[11px] uppercase tracking-[0.14em] text-areia/60">
                Target Loudness (LUFS)
              </label>
              <input
                type="number"
                step={0.5}
                min={-24}
                max={-8}
                value={targetLufsInput}
                onChange={(e) => setTargetLufsInput(e.target.value)}
                onBlur={() => setTargetLufsInput(String(parseClampedLufs()))}
                className="tabular-nums h-9 w-full rounded-xl border border-areia/12 bg-void/75 px-3 text-xs text-areia outline-none focus:border-laranja-criacao"
              />
            </div>
          </div>

          {activeJob && (
            <div className="space-y-2 rounded-xl border border-areia/12 bg-void/75 p-3">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-1.5 font-medium text-areia">
                  {activeJob.state === "completed" ? (
                    <CheckCircle2 className="h-4 w-4 text-amarelo-ideias" />
                  ) : activeJob.state === "failed" ? (
                    <XCircle className="h-4 w-4 text-vermelho-foco" />
                  ) : (
                    <Loader2 className="h-4 w-4 animate-spin text-laranja-criacao motion-reduce:animate-none" />
                  )}
                  <span>{activeJob.stage_label}</span>
                </div>
                <span className="tabular-nums text-[11px] text-amarelo-ideias">
                  {Math.round(activeJob.progress * 100)}%
                </span>
              </div>

              <div className="h-1.5 w-full overflow-hidden rounded-full bg-grafite">
                <div
                  className="h-full w-full origin-left rounded-full bg-gradient-to-r from-laranja-criacao to-amarelo-ideias transition-transform duration-300 motion-reduce:transition-none"
                  style={{ transform: `scaleX(${progressScale})` }}
                />
              </div>

              {activeJob.output_path && (
                <div className="flex items-center justify-between gap-2 pt-0.5">
                  <span className="truncate text-[10px] text-areia/70">
                    Saved: {activeJob.output_path}
                  </span>
                  {activeJob.state === "completed" &&
                    (desktopMode ? (
                      <button
                        type="button"
                        disabled={savingDesktop}
                        onClick={() => void handleDesktopSave()}
                        className="flex shrink-0 items-center gap-1 rounded-full border border-amarelo-ideias/45 bg-carbono px-2.5 py-1 text-[10px] font-medium text-amarelo-ideias hover:bg-accent disabled:opacity-50"
                      >
                        <FolderDown className="h-3 w-3" />
                        <span>{savingDesktop ? "Saving..." : "Save to Downloads"}</span>
                      </button>
                    ) : (
                      <a
                        href={client.getExportDownloadUrl(activeJob.id)}
                        download
                        className="shrink-0 rounded-full border border-amarelo-ideias/45 bg-carbono px-2.5 py-1 text-[10px] font-medium text-amarelo-ideias hover:bg-accent"
                      >
                        Download
                      </a>
                    ))}
                </div>
              )}
              {desktopSaveMsg && (
                <div className="rounded-lg border border-amarelo-ideias/35 bg-amarelo-ideias/10 p-2 text-[10px] text-areia">
                  {desktopSaveMsg}
                </div>
              )}
              {activeJob.error && (
                <div className="rounded-lg border border-vermelho-foco/40 bg-vermelho-foco/15 p-2 text-[10px] text-areia">
                  {activeJob.error}
                </div>
              )}
            </div>
          )}
        </div>

        <div className="mt-5 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="rounded-full border border-areia/14 px-4 py-2 text-xs text-areia/80 hover:bg-accent"
          >
            Close
          </button>
          <button
            type="button"
            disabled={submitting || isJobActive}
            onClick={() => void handleStart()}
            className="btn-solar-outline flex items-center gap-1.5 px-5 py-2 text-xs font-medium text-areia disabled:opacity-50"
          >
            <Download className="h-3.5 w-3.5 text-amarelo-ideias" />
            <span>
              {submitting
                ? "Starting..."
                : isJobActive
                  ? "Rendering..."
                  : "Render Video"}
            </span>
          </button>
        </div>
      </div>
    </div>
  );
}
