import { useEffect, useRef, useState } from "react";
import {
  CheckCircle2,
  Download,
  Film,
  FolderOpen,
  Loader2,
  X,
  XCircle,
} from "lucide-react";
import type {
  ExportFormat,
  ExportJobStatus,
  ExportQuality,
  SolarisEngineClient,
} from "@solaris/engine-client";
import { isDesktopTauri } from "../../lib/engineBridge";
import {
  BROWSER_DOWNLOAD_LOCATION,
  buildExportFileName,
  deliverExportToTarget,
  ensureExportWritePermission,
  exportExtensionForFormat,
  getRememberedExportFolder,
  pickExportDestination,
  sanitizeExportBaseName,
  setRememberedExportFolder,
  supportsExportDestinationPicker,
  type ExportDestinationTarget,
} from "../../lib/exportDelivery";
import { SolarSelect } from "../ui/SolarSelect";

interface ExportModalProps {
  open: boolean;
  projectId: string;
  projectName: string;
  client: SolarisEngineClient;
  activeJob: ExportJobStatus | null;
  onJobStarted: (job: ExportJobStatus) => void;
  onExportDelivered?: (savedPath: string, browserDownload: boolean) => void;
  onClose: () => void;
}

interface DeliveryState {
  jobId: string;
  status: "saving" | "saved" | "error";
  location?: string;
  browserDownload?: boolean;
  error?: string;
}

interface ArmedExportDelivery {
  jobId: string;
  target: ExportDestinationTarget | null;
  fileName: string;
}

export function ExportModal({
  open,
  projectId,
  projectName,
  client,
  activeJob,
  onJobStarted,
  onExportDelivered,
  onClose,
}: ExportModalProps) {
  const [format, setFormat] = useState<ExportFormat>("mp4");
  const [quality, setQuality] = useState<ExportQuality>("standard");
  const [targetLufsInput, setTargetLufsInput] = useState("-14");
  const [fileNameInput, setFileNameInput] = useState(() =>
    sanitizeExportBaseName(projectName),
  );
  const [selectedTarget, setSelectedTarget] = useState<ExportDestinationTarget | null>(() =>
    getRememberedExportFolder(),
  );
  const [pickingDestination, setPickingDestination] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [deliveryState, setDeliveryState] = useState<DeliveryState | null>(null);
  const [deliveryArmSeq, setDeliveryArmSeq] = useState(0);
  const [manualSaving, setManualSaving] = useState(false);

  const armedDeliveryRef = useRef<ArmedExportDelivery | null>(null);
  const autoDeliveredIdsRef = useRef<Set<string>>(new Set());
  const onExportDeliveredRef = useRef(onExportDelivered);
  onExportDeliveredRef.current = onExportDelivered;

  const isDesktop = isDesktopTauri();
  const destinationPickerAvailable = supportsExportDestinationPicker();

  const isJobActive =
    activeJob !== null &&
    (activeJob.state === "queued" ||
      activeJob.state === "rendering_gfx" ||
      activeJob.state === "compositing");

  useEffect(() => {
    setFileNameInput(sanitizeExportBaseName(projectName));
    setSelectedTarget((prev) => (prev?.kind === "fs_file_handle" ? null : prev));
  }, [projectId, projectName]);

  useEffect(() => {
    if (!open) return;
    setSubmitError(null);
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  // Automatically save/download the finished video to the chosen destination,
  // even if the Export modal was closed while the render ran in the background.
  useEffect(() => {
    if (!activeJob || activeJob.state !== "completed") return;
    const armed = armedDeliveryRef.current;
    if (!armed || armed.jobId !== activeJob.id) return;
    if (autoDeliveredIdsRef.current.has(activeJob.id)) return;

    autoDeliveredIdsRef.current.add(activeJob.id);
    const jobSnapshot = activeJob;
    const target = armed.target;
    const fileName = armed.fileName;

    setDeliveryState({
      jobId: jobSnapshot.id,
      status: "saving",
      location: target?.name ?? BROWSER_DOWNLOAD_LOCATION,
    });

    void deliverExportToTarget(client, jobSnapshot, fileName, target)
      .then((res) => {
        if (target?.kind === "fs_file_handle") {
          setSelectedTarget((prev) =>
            prev?.kind === "fs_file_handle" && prev.handle === target.handle
              ? null
              : prev,
          );
        }
        setDeliveryState({
          jobId: jobSnapshot.id,
          status: "saved",
          location: res.savedPath,
          browserDownload: res.browserDownload,
        });
        onExportDeliveredRef.current?.(res.savedPath, res.browserDownload);
      })
      .catch((err) => {
        setDeliveryState({
          jobId: jobSnapshot.id,
          status: "error",
          error:
            err instanceof Error
              ? err.message
              : "Failed to save rendered video to the chosen destination",
        });
      });
  }, [activeJob, client, deliveryArmSeq]);

  if (!open) return null;

  const parseClampedLufs = (): number => {
    const parsed = parseFloat(targetLufsInput);
    if (!Number.isFinite(parsed)) return -14;
    return Math.max(-24, Math.min(-8, parsed));
  };

  const handleChooseDestination = async () => {
    if (pickingDestination || isJobActive || !destinationPickerAvailable) return;
    setPickingDestination(true);
    setSubmitError(null);
    try {
      const picked = await pickExportDestination({
        suggestedBaseName: fileNameInput || projectName,
        format,
      });
      if (picked) {
        setSelectedTarget(picked);
        if (picked.kind === "desktop_path") {
          setRememberedExportFolder(picked);
        } else {
          setFileNameInput(sanitizeExportBaseName(picked.name));
        }
      }
    } catch (err) {
      setSubmitError(
        err instanceof Error
          ? `${err.message}. You can still render using Browser Downloads.`
          : "Could not open save dialog. You can still render using Browser Downloads.",
      );
    } finally {
      setPickingDestination(false);
    }
  };

  const handleClearWebDestination = () => {
    if (isJobActive) return;
    setSelectedTarget(null);
    setSubmitError(null);
  };

  const handleStart = async () => {
    if (submitting || isJobActive || pickingDestination) return;
    setSubmitError(null);

    let target = selectedTarget;
    if (isDesktop && !target) {
      setPickingDestination(true);
      try {
        target = await pickExportDestination({
          suggestedBaseName: fileNameInput || projectName,
          format,
        });
      } catch (err) {
        setSubmitError(
          err instanceof Error ? err.message : "Could not open folder picker",
        );
        setPickingDestination(false);
        return;
      } finally {
        setPickingDestination(false);
      }
      if (!target) {
        return;
      }
      setSelectedTarget(target);
      setRememberedExportFolder(target);
    } else if (target) {
      try {
        const permitted = await ensureExportWritePermission(target);
        if (!permitted) {
          setSubmitError(
            "Write permission for the chosen destination was denied. Choose a location again or reset to Browser Downloads.",
          );
          return;
        }
      } catch (err) {
        setSubmitError(
          err instanceof Error
            ? err.message
            : "Could not verify write permission for the chosen destination.",
        );
        return;
      }
    }

    const clampedLufs = parseClampedLufs();
    setTargetLufsInput(String(clampedLufs));
    const cleanBaseName = sanitizeExportBaseName(fileNameInput || projectName);
    setFileNameInput(cleanBaseName);
    const desiredFileName = buildExportFileName(cleanBaseName, format);

    setSubmitting(true);
    setDeliveryState(null);
    try {
      const job = await client.startExport(projectId, {
        format,
        quality,
        target_lufs: clampedLufs,
        output_name: cleanBaseName,
      });
      armedDeliveryRef.current = {
        jobId: job.id,
        target,
        fileName: desiredFileName,
      };
      setDeliveryArmSeq((s) => s + 1);
      onJobStarted(job);
    } catch (err) {
      setSubmitError(err instanceof Error ? err.message : "Failed to start export");
    } finally {
      setSubmitting(false);
    }
  };

  const handleManualSave = async () => {
    if (!activeJob || activeJob.state !== "completed" || manualSaving) return;
    setManualSaving(true);
    setSubmitError(null);

    let target = selectedTarget;
    if (isDesktop && !target) {
      try {
        target = await pickExportDestination({
          suggestedBaseName: fileNameInput || projectName,
          format,
        });
      } catch (err) {
        setSubmitError(
          err instanceof Error ? err.message : "Could not open folder picker",
        );
        setManualSaving(false);
        return;
      }
      if (!target) {
        setManualSaving(false);
        return;
      }
      setSelectedTarget(target);
      setRememberedExportFolder(target);
    } else if (target) {
      const permitted = await ensureExportWritePermission(target).catch(() => false);
      if (!permitted) {
        setSubmitError(
          "Write permission for the chosen destination was denied. Choose a location again or reset to Browser Downloads.",
        );
        setManualSaving(false);
        return;
      }
    }

    const desiredFileName = buildExportFileName(
      fileNameInput || projectName,
      format,
      activeJob.output_path,
    );
    setDeliveryState({
      jobId: activeJob.id,
      status: "saving",
      location: target?.name ?? BROWSER_DOWNLOAD_LOCATION,
    });

    try {
      const res = await deliverExportToTarget(
        client,
        activeJob,
        desiredFileName,
        target,
      );
      if (target?.kind === "fs_file_handle") {
        setSelectedTarget((prev) =>
          prev?.kind === "fs_file_handle" && prev.handle === target.handle
            ? null
            : prev,
        );
      }
      setDeliveryState({
        jobId: activeJob.id,
        status: "saved",
        location: res.savedPath,
        browserDownload: res.browserDownload,
      });
      onExportDeliveredRef.current?.(res.savedPath, res.browserDownload);
    } catch (err) {
      setDeliveryState({
        jobId: activeJob.id,
        status: "error",
        error: err instanceof Error ? err.message : "Failed to download rendered video",
      });
    } finally {
      setManualSaving(false);
    }
  };

  const progressScale = Math.max(0, Math.min(1, activeJob?.progress ?? 0));
  const currentDelivery =
    activeJob && deliveryState?.jobId === activeJob.id ? deliveryState : null;
  const activeExt = exportExtensionForFormat(format);
  const destinationDisplayPath =
    selectedTarget?.kind === "desktop_path"
      ? selectedTarget.path
      : selectedTarget?.name;

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="export-modal-title"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
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
              {isDesktop ? "Output Folder" : "Save Destination"}
            </label>
            <div className="flex items-center justify-between gap-2.5 rounded-xl border border-areia/14 bg-void/75 px-3 py-2">
              <div className="flex min-w-0 flex-1 items-center gap-2">
                <FolderOpen
                  className={`h-4 w-4 shrink-0 ${
                    selectedTarget || !isDesktop
                      ? "text-amarelo-ideias"
                      : "text-areia/45"
                  }`}
                />
                <div className="min-w-0 flex-1">
                  {selectedTarget ? (
                    <>
                      <div
                        className="truncate text-xs font-medium text-areia"
                        title={destinationDisplayPath}
                      >
                        {selectedTarget.name}
                      </div>
                      <div
                        className="truncate text-[10px] text-areia/55"
                        title={destinationDisplayPath}
                      >
                        {selectedTarget.kind === "desktop_path"
                          ? selectedTarget.path
                          : "Saves directly to chosen file after render"}
                      </div>
                    </>
                  ) : isDesktop ? (
                    <>
                      <div className="truncate text-xs text-areia/80">
                        Choose destination folder
                      </div>
                      <div className="truncate text-[10px] text-areia/50">
                        Video saves automatically when rendering completes
                      </div>
                    </>
                  ) : (
                    <>
                      <div className="truncate text-xs font-medium text-areia">
                        Browser Downloads
                      </div>
                      <div className="truncate text-[10px] text-areia/55">
                        Auto-downloads when render finishes (or choose any save location)
                      </div>
                    </>
                  )}
                </div>
              </div>
              {destinationPickerAvailable && (
                <div className="flex shrink-0 items-center gap-1.5">
                  {!isDesktop && selectedTarget?.kind === "fs_file_handle" && (
                    <button
                      type="button"
                      disabled={pickingDestination || isJobActive}
                      onClick={handleClearWebDestination}
                      className="rounded-full border border-areia/14 px-2 py-1 text-[10px] text-areia/65 transition-colors hover:border-areia/30 hover:text-areia disabled:opacity-50"
                      title="Use default Browser Downloads"
                    >
                      Reset
                    </button>
                  )}
                  <button
                    type="button"
                    disabled={pickingDestination || isJobActive}
                    onClick={() => void handleChooseDestination()}
                    className="rounded-full border border-areia/16 bg-carbono px-3 py-1 text-[11px] font-medium text-areia transition-colors hover:border-amarelo-ideias/45 hover:bg-accent disabled:opacity-50 motion-reduce:transition-none"
                  >
                    {pickingDestination
                      ? "Opening..."
                      : selectedTarget
                        ? "Change"
                        : isDesktop
                          ? "Choose"
                          : "Save As..."}
                  </button>
                </div>
              )}
            </div>
          </div>

          <div>
            <label className="mb-1.5 block text-[11px] uppercase tracking-[0.14em] text-areia/60">
              File Name
            </label>
            <div className="flex h-9 items-center overflow-hidden rounded-xl border border-areia/14 bg-void/75 focus-within:border-laranja-criacao">
              <input
                type="text"
                disabled={isJobActive}
                value={fileNameInput}
                onChange={(e) => {
                  setFileNameInput(e.target.value);
                  if (selectedTarget?.kind === "fs_file_handle") {
                    setSelectedTarget(null);
                  }
                }}
                onBlur={() =>
                  setFileNameInput(sanitizeExportBaseName(fileNameInput || projectName))
                }
                placeholder="solaris-export"
                className="h-full min-w-0 flex-1 bg-transparent px-3 text-xs text-areia outline-none disabled:opacity-60"
              />
              <span className="shrink-0 border-l border-areia/10 bg-carbono/60 px-2.5 py-2 text-[11px] font-medium text-areia/65">
                .{activeExt}
              </span>
            </div>
          </div>

          <div>
            <label className="mb-1.5 block text-[11px] uppercase tracking-[0.14em] text-areia/60">
              Format
            </label>
            <SolarSelect
              variant="field"
              wrapperClassName="w-full"
              disabled={isJobActive}
              value={format}
              onChange={(e) => {
                setFormat(e.target.value as ExportFormat);
                if (selectedTarget?.kind === "fs_file_handle") {
                  setSelectedTarget(null);
                }
              }}
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
            </SolarSelect>
          </div>

          <div className="grid grid-cols-2 items-end gap-3">
            <div>
              <label className="mb-1.5 block text-[11px] uppercase tracking-[0.14em] text-areia/60">
                Quality
              </label>
              <SolarSelect
                variant="field"
                wrapperClassName="w-full"
                disabled={isJobActive}
                value={quality}
                onChange={(e) => setQuality(e.target.value as ExportQuality)}
              >
                <option value="draft" className="bg-carbono">
                  Draft (Fast)
                </option>
                <option value="standard" className="bg-carbono">
                  Standard
                </option>
                <option value="high" className="bg-carbono">
                  High Master
                </option>
              </SolarSelect>
            </div>

            <div>
              <label className="mb-1.5 block text-[11px] uppercase tracking-[0.14em] text-areia/60">
                Target LUFS
              </label>
              <input
                type="number"
                step={0.5}
                min={-24}
                max={-8}
                disabled={isJobActive}
                value={targetLufsInput}
                onChange={(e) => setTargetLufsInput(e.target.value)}
                onBlur={() => setTargetLufsInput(String(parseClampedLufs()))}
                className="tabular-nums h-9 w-full rounded-xl border border-areia/14 bg-void/75 px-3 text-xs text-areia outline-none focus:border-laranja-criacao disabled:opacity-60"
              />
            </div>
          </div>

          {activeJob && (
            <div className="space-y-2 rounded-xl border border-areia/12 bg-void/75 p-3">
              <div className="flex items-center justify-between gap-2">
                <div className="flex min-w-0 flex-1 items-center gap-1.5 font-medium text-areia">
                  {activeJob.state === "completed" ? (
                    <CheckCircle2 className="h-4 w-4 shrink-0 text-amarelo-ideias" />
                  ) : activeJob.state === "failed" ? (
                    <XCircle className="h-4 w-4 shrink-0 text-vermelho-foco" />
                  ) : (
                    <Loader2 className="h-4 w-4 shrink-0 animate-spin text-laranja-criacao motion-reduce:animate-none" />
                  )}
                  <span className="truncate" title={activeJob.stage_label}>
                    {activeJob.stage_label}
                  </span>
                </div>
                <span className="tabular-nums shrink-0 text-[11px] text-amarelo-ideias">
                  {Math.round(activeJob.progress * 100)}%
                </span>
              </div>

              <div className="h-1.5 w-full overflow-hidden rounded-full bg-grafite">
                <div
                  className="h-full w-full origin-left rounded-full bg-gradient-to-r from-laranja-criacao to-amarelo-ideias transition-transform duration-300 motion-reduce:transition-none"
                  style={{ transform: `scaleX(${progressScale})` }}
                />
              </div>

              {currentDelivery?.status === "saving" && (
                <div className="flex items-center gap-1.5 pt-0.5 text-[11px] text-areia/80">
                  <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-amarelo-ideias motion-reduce:animate-none" />
                  <span className="truncate">
                    Saving video to {currentDelivery.location}...
                  </span>
                </div>
              )}

              {currentDelivery?.status === "saved" && (
                <div className="flex items-center justify-between gap-2 rounded-lg border border-amarelo-ideias/35 bg-amarelo-ideias/10 px-2.5 py-2 text-[11px] text-areia">
                  <span
                    className="min-w-0 flex-1 truncate"
                    title={currentDelivery.location}
                  >
                    {currentDelivery.browserDownload ? "Downloaded to" : "Saved to"}{" "}
                    {currentDelivery.location}
                  </span>
                  <button
                    type="button"
                    disabled={manualSaving}
                    onClick={() => void handleManualSave()}
                    className="shrink-0 rounded-full border border-amarelo-ideias/45 bg-carbono px-2.5 py-0.5 text-[10px] font-medium text-amarelo-ideias hover:bg-accent disabled:opacity-50"
                  >
                    {manualSaving ? "Saving..." : "Save Again"}
                  </button>
                </div>
              )}

              {currentDelivery?.status === "error" && (
                <div className="space-y-1.5 rounded-lg border border-vermelho-foco/40 bg-vermelho-foco/15 p-2 text-[11px] text-areia">
                  <div className="break-words">{currentDelivery.error}</div>
                  <button
                    type="button"
                    disabled={manualSaving}
                    onClick={() => void handleManualSave()}
                    className="rounded-full border border-areia/25 bg-carbono px-2.5 py-0.5 text-[10px] font-medium text-areia hover:bg-accent disabled:opacity-50"
                  >
                    {manualSaving ? "Retrying..." : "Retry Download"}
                  </button>
                </div>
              )}

              {activeJob.state === "completed" && !currentDelivery && (
                <div className="flex items-center justify-between gap-2 pt-0.5">
                  <span
                    className="min-w-0 flex-1 truncate text-[10px] text-areia/65"
                    title={activeJob.output_path ?? undefined}
                  >
                    Ready to download
                  </span>
                  <button
                    type="button"
                    disabled={manualSaving}
                    onClick={() => void handleManualSave()}
                    className="shrink-0 rounded-full border border-amarelo-ideias/45 bg-carbono px-2.5 py-1 text-[10px] font-medium text-amarelo-ideias hover:bg-accent disabled:opacity-50"
                  >
                    {manualSaving ? "Saving..." : "Save Video"}
                  </button>
                </div>
              )}

              {activeJob.error && (
                <div className="break-words rounded-lg border border-vermelho-foco/40 bg-vermelho-foco/15 p-2 text-[10px] text-areia">
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
            disabled={submitting || isJobActive || pickingDestination}
            onClick={() => void handleStart()}
            className="btn-solar-outline flex items-center gap-1.5 px-5 py-2 text-xs font-medium text-areia disabled:opacity-50"
          >
            <Download className="h-3.5 w-3.5 text-amarelo-ideias" />
            <span>
              {submitting
                ? "Starting..."
                : pickingDestination
                  ? "Choosing Location..."
                  : isJobActive
                    ? "Rendering..."
                    : isDesktop && !selectedTarget
                      ? "Choose Folder & Render"
                      : selectedTarget
                        ? "Render & Save"
                        : "Render & Download"}
            </span>
          </button>
        </div>
      </div>
    </div>
  );
}
