export interface RulerTick {
  sec: number;
  pct: number;
  major: boolean;
  edge: "start" | "mid" | "end";
}

export interface RulerTicksResult {
  majorStep: number;
  ticks: RulerTick[];
}

/**
 * Computes edge-safe major and minor ruler ticks adapted to timeline span and zoom.
 * Guarantees 0 <= pct <= 100 and marks edge ticks so labels never overflow horizontally.
 */
export function computeRulerTicks(
  timelineSpan: number,
  zoom: number,
): RulerTicksResult {
  const safeSpan = Number.isFinite(timelineSpan) && timelineSpan > 0 ? timelineSpan : 20;
  const safeZoom = Number.isFinite(zoom) && zoom > 0 ? zoom : 1;
  const effectiveSpan = safeSpan / safeZoom;

  let majorStep = 10;
  if (effectiveSpan <= 8) majorStep = 1;
  else if (effectiveSpan <= 25) majorStep = 2;
  else if (effectiveSpan <= 60) majorStep = 5;
  else if (effectiveSpan <= 140) majorStep = 10;
  else majorStep = 20;

  const subdivisions = majorStep === 1 ? 2 : 4;
  const minorStep = majorStep / subdivisions;
  const totalSteps = Math.floor(safeSpan / minorStep + 1e-6);
  const ticks: RulerTick[] = [];

  for (let i = 0; i <= totalSteps; i++) {
    const sec = Math.round(i * minorStep * 1000) / 1000;
    if (sec > safeSpan + 1e-6) break;
    const pct = Math.min(100, Math.max(0, (sec / safeSpan) * 100));
    const major = i % subdivisions === 0;
    const edge: "start" | "mid" | "end" =
      pct <= 2 ? "start" : pct >= 96 ? "end" : "mid";
    ticks.push({ sec, pct, major, edge });
  }

  return { majorStep, ticks };
}

export function formatCompactDuration(sec: number): string {
  const safe = Math.max(0, Number.isFinite(sec) ? sec : 0);
  if (safe < 60) {
    return `${safe.toFixed(1)}s`;
  }
  const mins = Math.floor(safe / 60);
  const rem = Math.round(safe % 60);
  return `${mins}m ${String(rem).padStart(2, "0")}s`;
}

export function snapTimeSec(
  sec: number,
  enabled: boolean,
  gridSec = 0.1,
): number {
  const safe = Math.max(0, Number.isFinite(sec) ? sec : 0);
  if (!enabled || !Number.isFinite(gridSec) || gridSec <= 0) {
    return Math.round(safe * 1000) / 1000;
  }
  return Math.round(Math.round(safe / gridSec) * gridSec * 1000) / 1000;
}
