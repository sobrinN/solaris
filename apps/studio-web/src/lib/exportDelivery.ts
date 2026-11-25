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

interface DirectoryPickerOptions {
  id?: string;
  mode?: "read" | "readwrite";
  startIn?: "desktop" | "documents" | "downloads" | "music" | "pictures" | "videos";
}

interface PermissionDescriptor {
  mode?: "read" | "readwrite";
}

interface WritableFileStreamLike extends WritableStream<Uint8Array> {
  write(data: BufferSource | Blob | string): Promise<void>;
  close(): Promise<void>;
  abort(reason?: unknown): Promise<void>;
}

interface FileHandleLike {
  createWritable(): Promise<WritableFileStreamLike>;
}

export interface ExportDirectoryHandle {
  readonly name: string;
  getFileHandle(name: string, options?: { create?: boolean }): Promise<FileHandleLike>;
  queryPermission?: (desc?: PermissionDescriptor) => Promise<PermissionState>;
  requestPermission?: (desc?: PermissionDescriptor) => Promise<PermissionState>;
}

type WindowWithDirectoryPicker = Window & {
  showDirectoryPicker?: (options?: DirectoryPickerOptions) => Promise<ExportDirectoryHandle>;
};

export type ExportFolderTarget =
  | { kind: "desktop_path"; name: string; path: string }
  | { kind: "fs_handle"; name: string; handle: ExportDirectoryHandle };

export const BROWSER_DOWNLOAD_LOCATION = "your browser's download folder";

export interface ExportDeliveryResult {
  savedPath: string;
  fileName: string;
  browserDownload: boolean;
}

let rememberedExportFolder: ExportFolderTarget | null = null;

export function getRememberedExportFolder(): ExportFolderTarget | null {
  return rememberedExportFolder;
}

export function setRememberedExportFolder(target: ExportFolderTarget | null): void {
  rememberedExportFolder = target;
}

export function supportsExportFolderPicker(): boolean {
  if (isDesktopTauri()) return true;
  if (typeof window === "undefined") return false;
  return typeof (window as WindowWithDirectoryPicker).showDirectoryPicker === "function";
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

export async function pickExportFolder(): Promise<ExportFolderTarget | null> {
  if (isDesktopTauri()) {
    const picked = await pickDesktopExportFolder();
    if (!picked) return null;
    const target: ExportFolderTarget = {
      kind: "desktop_path",
      name: folderBaseName(picked),
      path: picked,
    };
    setRememberedExportFolder(target);
    return target;
  }

  const picker =
    typeof window !== "undefined"
      ? (window as WindowWithDirectoryPicker).showDirectoryPicker
      : undefined;
  if (!picker) return null;

  try {
    const handle = await picker({
      id: "solaris-exports",
      mode: "readwrite",
      startIn: "downloads",
    });
    const target: ExportFolderTarget = {
      kind: "fs_handle",
      name: handle.name || "Selected Folder",
      handle,
    };
    setRememberedExportFolder(target);
    return target;
  } catch (err) {
    if (err instanceof DOMException && err.name === "AbortError") {
      return null;
    }
    throw err;
  }
}

export async function ensureFolderWritePermission(
  target: ExportFolderTarget,
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

async function resolveUniqueFileHandle(
  dir: ExportDirectoryHandle,
  desiredName: string,
): Promise<{ fileName: string; fileHandle: FileHandleLike }> {
  const dotIdx = desiredName.lastIndexOf(".");
  const stem = dotIdx > 0 ? desiredName.slice(0, dotIdx) : desiredName;
  const ext = dotIdx > 0 ? desiredName.slice(dotIdx) : "";

  for (let counter = 0; counter < 1000; counter += 1) {
    const candidate = counter === 0 ? desiredName : `${stem}-${counter}${ext}`;
    try {
      await dir.getFileHandle(candidate, { create: false });
      // File already exists — keep searching for a unique name
    } catch (err) {
      if (err instanceof DOMException && err.name === "NotFoundError") {
        const fileHandle = await dir.getFileHandle(candidate, { create: true });
        return { fileName: candidate, fileHandle };
      }
      throw err;
    }
  }

  const fallbackName = `${stem}-${Date.now()}${ext}`;
  const fileHandle = await dir.getFileHandle(fallbackName, { create: true });
  return { fileName: fallbackName, fileHandle };
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
  target: ExportFolderTarget | null,
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

  if (target?.kind === "fs_handle") {
    const { fileName, fileHandle } = await resolveUniqueFileHandle(
      target.handle,
      desiredFileName,
    );
    const writable = await fileHandle.createWritable();
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
    return {
      savedPath: `${target.name}/${fileName}`,
      fileName,
      browserDownload: false,
    };
  }

  return triggerBrowserBlobDownload(res, desiredFileName);
}
