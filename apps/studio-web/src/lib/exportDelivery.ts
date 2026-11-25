import type {
  ExportFormat,
  ExportJobStatus,
  SolarisEngineClient,
} from "@solaris/engine-client";
import {
  isDesktopTauri,
  pickDesktopExportFolder,
  saveDesktopExportFile,
} from "./engineBridge";

interface SaveFilePickerAcceptType {
  description?: string;
  accept: Record<string, string[]>;
}

interface SaveFilePickerOptions {
  id?: string;
  suggestedName?: string;
  startIn?: "desktop" | "documents" | "downloads" | "music" | "pictures" | "videos";
  types?: SaveFilePickerAcceptType[];
  excludeAcceptAllOption?: boolean;
}

interface PermissionDescriptor {
  mode?: "read" | "readwrite";
}

interface WritableFileStreamLike extends WritableStream<Uint8Array> {
  write(data: BufferSource | Blob | string): Promise<void>;
  close(): Promise<void>;
  abort(reason?: unknown): Promise<void>;
}

export interface ExportFileHandle {
  readonly name: string;
  createWritable(): Promise<WritableFileStreamLike>;
  queryPermission?: (desc?: PermissionDescriptor) => Promise<PermissionState>;
  requestPermission?: (desc?: PermissionDescriptor) => Promise<PermissionState>;
}

type WindowWithSaveFilePicker = Window & {
  showSaveFilePicker?: (options?: SaveFilePickerOptions) => Promise<ExportFileHandle>;
};

export type ExportDestinationTarget =
  | { kind: "desktop_path"; name: string; path: string }
  | { kind: "fs_file_handle"; name: string; handle: ExportFileHandle };

export const BROWSER_DOWNLOAD_LOCATION = "your browser's download folder";

export interface ExportDeliveryResult {
  savedPath: string;
  fileName: string;
  browserDownload: boolean;
}

let rememberedDesktopFolder: Extract<ExportDestinationTarget, { kind: "desktop_path" }> | null =
  null;

export function getRememberedExportFolder(): ExportDestinationTarget | null {
  return rememberedDesktopFolder;
}

export function setRememberedExportFolder(target: ExportDestinationTarget | null): void {
  rememberedDesktopFolder = target?.kind === "desktop_path" ? target : null;
}

export function supportsExportDestinationPicker(): boolean {
  if (isDesktopTauri()) return true;
  if (typeof window === "undefined") return false;
  return typeof (window as WindowWithSaveFilePicker).showSaveFilePicker === "function";
}

function folderBaseName(rawPath: string): string {
  const normalized = rawPath.replace(/[\\/]+$/, "");
  const parts = normalized.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? rawPath;
}

export function exportExtensionForFormat(
  format: ExportFormat,
  outputPath?: string | null,
): string {
  const match = outputPath?.match(/\.([a-zA-Z0-9]{2,5})$/);
  if (match?.[1]) return match[1].toLowerCase();
  if (format === "mov_pro_res4444") return "mov";
  if (format === "webm_alpha") return "webm";
  return "mp4";
}

function savePickerTypesForFormat(format: ExportFormat): SaveFilePickerAcceptType[] {
  if (format === "mov_pro_res4444") {
    return [
      {
        description: "QuickTime Movie (ProRes 4444)",
        accept: { "video/quicktime": [".mov"] },
      },
    ];
  }
  if (format === "webm_alpha") {
    return [
      {
        description: "WebM Video",
        accept: { "video/webm": [".webm"] },
      },
    ];
  }
  return [
    {
      description: "MP4 Video",
      accept: { "video/mp4": [".mp4"] },
    },
  ];
}

export function sanitizeExportBaseName(raw: string): string {
  const stripped = raw
    .trim()
    .replace(/\.[a-zA-Z0-9]{2,5}$/, "")
    .replace(/[<>:"/\\|?*\u0000-\u001F]+/g, "_")
    .replace(/\s+/g, "_")
    .replace(/^_+|_+$/g, "")
    .slice(0, 96);
  return stripped || "solaris-export";
}

export function buildExportFileName(
  baseName: string,
  format: ExportFormat,
  outputPath?: string | null,
): string {
  const cleanBase = sanitizeExportBaseName(baseName);
  const ext = exportExtensionForFormat(format, outputPath);
  return `${cleanBase}.${ext}`;
}

// On the web we use `showSaveFilePicker` (single-file Save As) instead of
// `showDirectoryPicker`, because Chromium blocks `showDirectoryPicker` on standard
// user folders (Downloads, Desktop, Documents, Movies, Home) with a misleading
// "contains system files" error, whereas `showSaveFilePicker` is allowed in all of them.
export async function pickExportDestination(options: {
  suggestedBaseName: string;
  format: ExportFormat;
}): Promise<ExportDestinationTarget | null> {
  if (isDesktopTauri()) {
    const picked = await pickDesktopExportFolder();
    if (!picked) return null;
    const target: ExportDestinationTarget = {
      kind: "desktop_path",
      name: folderBaseName(picked),
      path: picked,
    };
    setRememberedExportFolder(target);
    return target;
  }

  const picker =
    typeof window !== "undefined"
      ? (window as WindowWithSaveFilePicker).showSaveFilePicker
      : undefined;
  if (!picker) return null;

  const suggestedName = buildExportFileName(options.suggestedBaseName, options.format);

  try {
    const handle = await picker({
      id: "solaris-exports",
      suggestedName,
      startIn: "downloads",
      types: savePickerTypesForFormat(options.format),
    });
    return {
      kind: "fs_file_handle",
      name: handle.name || suggestedName,
      handle,
    };
  } catch (err) {
    if (err instanceof DOMException && err.name === "AbortError") {
      return null;
    }
    throw err;
  }
}

export async function ensureExportWritePermission(
  target: ExportDestinationTarget,
): Promise<boolean> {
  if (target.kind === "desktop_path") return true;
  const { handle } = target;
  if (typeof handle.queryPermission === "function") {
    const current = await handle.queryPermission({ mode: "readwrite" });
    if (current === "granted") return true;
  }
  if (typeof handle.requestPermission === "function") {
    const requested = await handle.requestPermission({ mode: "readwrite" });
    return requested === "granted";
  }
  return true;
}

// Holds the whole export in memory on purpose: a direct link would stream to disk, but it
// cannot report errors in-app, and the server's Content-Disposition name overrides `download`.
async function triggerBrowserBlobDownload(
  res: Response,
  fileName: string,
): Promise<ExportDeliveryResult> {
  const blob = await res.blob();
  const objectUrl = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = objectUrl;
  anchor.download = fileName;
  anchor.style.display = "none";
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  setTimeout(() => URL.revokeObjectURL(objectUrl), 60_000);
  return {
    savedPath: BROWSER_DOWNLOAD_LOCATION,
    fileName,
    browserDownload: true,
  };
}

export async function deliverExportToTarget(
  client: SolarisEngineClient,
  job: ExportJobStatus,
  desiredFileName: string,
  target: ExportDestinationTarget | null,
): Promise<ExportDeliveryResult> {
  if (target?.kind === "desktop_path" && job.output_path) {
    const saved = await saveDesktopExportFile(
      job.output_path,
      desiredFileName,
      target.path,
    );
    if (!saved) {
      throw new Error("Desktop export save is unavailable");
    }
    return {
      savedPath: saved,
      fileName: folderBaseName(saved),
      browserDownload: false,
    };
  }

  if (isDesktopTauri() && job.output_path) {
    const saved = await saveDesktopExportFile(job.output_path, desiredFileName);
    if (!saved) {
      throw new Error("Desktop export save is unavailable");
    }
    return {
      savedPath: saved,
      fileName: folderBaseName(saved),
      browserDownload: false,
    };
  }

  const res = await client.fetchExportDownload(job.id);

  if (target?.kind === "fs_file_handle") {
    const writable = await target.handle.createWritable();
    if (res.body && typeof res.body.pipeTo === "function") {
      await res.body.pipeTo(writable);
    } else {
      try {
        const blob = await res.blob();
        await writable.write(blob);
        await writable.close();
      } catch (err) {
        await writable.abort(err).catch(() => {});
        throw err;
      }
    }
    const savedName = target.name || desiredFileName;
    return {
      savedPath: savedName,
      fileName: savedName,
      browserDownload: false,
    };
  }

  return triggerBrowserBlobDownload(res, desiredFileName);
}
