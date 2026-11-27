import type { AspectPreset, CanvasConfig, MediaAsset } from "@solaris/engine-client";

const STANDARD_FPS = [24, 25, 30, 50, 60] as const;

export function snapStandardFps(rawFps?: number | null): number {
  if (!rawFps || !Number.isFinite(rawFps) || rawFps <= 1) return 30;
  let best = 30;
  let bestDiff = Infinity;
  for (const candidate of STANDARD_FPS) {
    const diff = Math.abs(rawFps - candidate);
    if (diff < bestDiff) {
      bestDiff = diff;
      best = candidate;
    }
  }
  return best;
}

export function inferAspectFromDimensions(width: number, height: number): AspectPreset {
  const ratio = width / Math.max(1, height);
  if (ratio > 1.15) return "landscape16x9";
  if (ratio < 0.88) return "vertical9x16";
  return "square1x1";
}

export function canvasConfigForPreset(preset: AspectPreset, fpsNum = 30): CanvasConfig {
  const dims =
    preset === "landscape16x9"
      ? { width: 1920, height: 1080 }
      : preset === "square1x1"
        ? { width: 1080, height: 1080 }
        : { width: 1080, height: 1920 };
  return {
    ...dims,
    fps_num: fpsNum,
    fps_den: 1,
    aspect_preset: preset,
  };
}

export function inferCanvasFromVideo(media: MediaAsset): CanvasConfig | null {
  if (media.kind !== "video" || !media.intelligence.analyzed) return null;
  const w = media.intelligence.width ?? 0;
  const h = media.intelligence.height ?? 0;
  if (w < 16 || h < 16) return null;
  const preset = inferAspectFromDimensions(w, h);
  const fps = snapStandardFps(media.intelligence.fps);
  return canvasConfigForPreset(preset, fps);
}
