const MAX_GFX_ID_LENGTH = 64;
const MAX_GFX_NAME_LENGTH = 80;

// Mirrors starter_code_assets() in crates/server/src/state.rs.
export const STARTER_GFX_IDS = [
  "chapter-title-solar",
  "stat-card-solar",
  "comment-bubble-solar",
  "lower-third-solar",
  "progress-bar-solar",
] as const;

export interface Point {
  x: number;
  y: number;
}

export interface Size {
  width: number;
  height: number;
}

export type GfxPreviewAspect = "16:9" | "9:16" | "1:1";

export interface GfxPreviewGeometry {
  boxWidthPx: number;
  boxHeightPx: number;
  viewportWidthPx: number;
  viewportHeightPx: number;
  scale: number;
}

export type GfxNameCheck = { ok: true; name: string } | { ok: false; message: string };

const COPY_SUFFIX = " (copy)";
const PREVIEW_HEIGHT_PX = 64;
// Canonical virtual viewports that GFX scenes are authored for.
const VIRTUAL_VIEWPORT: Record<GfxPreviewAspect, Size> = {
  "16:9": { width: 960, height: 540 },
  "9:16": { width: 540, height: 960 },
  "1:1": { width: 720, height: 720 },
};

export function missingStarterGfxIds(assets: readonly { id: string }[]): string[] {
  const present = new Set(assets.map((asset) => asset.id));
  return STARTER_GFX_IDS.filter((id) => !present.has(id));
}

export function nextDuplicateId(sourceId: string, takenIds: Iterable<string>): string {
  const taken = new Set(takenIds);
  for (let copy = 1; ; copy += 1) {
    const suffix = copy === 1 ? "-copy" : `-copy-${copy}`;
    const candidate = `${sourceId.slice(0, MAX_GFX_ID_LENGTH - suffix.length)}${suffix}`;
    if (!taken.has(candidate)) return candidate;
  }
}

export function duplicateGfxName(name: string): string {
  const base = name.trim().slice(0, MAX_GFX_NAME_LENGTH - COPY_SUFFIX.length).trimEnd();
  return `${base}${COPY_SUFFIX}`;
}

export function checkGfxName(raw: string): GfxNameCheck {
  const name = raw.trim();
  if (name.length === 0) return { ok: false, message: "Enter a name." };
  if (name.length > MAX_GFX_NAME_LENGTH) {
    return { ok: false, message: `Use ${MAX_GFX_NAME_LENGTH} characters or fewer.` };
  }
  return { ok: true, name };
}

export function clampToViewport(origin: Point, size: Size, viewport: Size, margin = 8): Point {
  const maxX = Math.max(margin, viewport.width - size.width - margin);
  const maxY = Math.max(margin, viewport.height - size.height - margin);
  return {
    x: Math.min(Math.max(origin.x, margin), maxX),
    y: Math.min(Math.max(origin.y, margin), maxY),
  };
}

export function gfxPreviewGeometry(aspect: GfxPreviewAspect): GfxPreviewGeometry {
  const viewport = VIRTUAL_VIEWPORT[aspect];
  const scale = PREVIEW_HEIGHT_PX / viewport.height;
  return {
    boxWidthPx: viewport.width * scale,
    boxHeightPx: PREVIEW_HEIGHT_PX,
    viewportWidthPx: viewport.width,
    viewportHeightPx: viewport.height,
    scale,
  };
}

export function errorMessage(err: unknown, fallback: string): string {
  return err instanceof Error && err.message ? err.message : fallback;
}
