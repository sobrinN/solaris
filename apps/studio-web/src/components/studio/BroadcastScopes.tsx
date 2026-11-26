import { useCallback, useEffect, useImperativeHandle, useRef, useState, type Ref } from "react";
import type { ColorGrade } from "@solaris/engine-client";

export interface SampledPixelFrame {
  width: number;
  height: number;
  data: Uint8ClampedArray;
}

export interface BroadcastScopesHandle {
  drawPixelFrame: (frame: SampledPixelFrame | null) => void;
}

interface BroadcastScopesProps {
  scopesRef?: Ref<BroadcastScopesHandle>;
  grade: ColorGrade;
  playheadSec: number;
  integratedLufs?: number | null;
  voiceLevelPct?: number;
  musicLevelPct?: number;
}

export function BroadcastScopes({
  scopesRef,
  grade,
  playheadSec,
  integratedLufs,
  voiceLevelPct = 0,
  musicLevelPct = 0,
}: BroadcastScopesProps) {
  const waveRef = useRef<HTMLCanvasElement | null>(null);
  const vecRef = useRef<HTMLCanvasElement | null>(null);
  const lastFrameRef = useRef<SampledPixelFrame | null>(null);
  const playheadRef = useRef<number>(playheadSec);
  playheadRef.current = playheadSec;
  const gradeRef = useRef<ColorGrade>(grade);
  gradeRef.current = grade;
  const [hasLiveFrame, setHasLiveFrame] = useState(false);

  const drawWaveform = useCallback((frame: SampledPixelFrame | null) => {
    const canvas = waveRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const w = canvas.width;
    const h = canvas.height;
    const padL = 22;
    const padR = 4;
    const padY = 6;
    const plotW = w - padL - padR;
    const plotH = h - padY * 2;
    ctx.clearRect(0, 0, w, h);

    // Broadcast IRE graticule (100, 75, 50, 25, 0)
    ctx.font = '600 8px "Sora Variable", "Sora", sans-serif';
    ctx.textAlign = "right";
    ctx.textBaseline = "middle";
    for (const [ire, frac] of [
      ["100", 0],
      ["75", 0.25],
      ["50", 0.5],
      ["25", 0.75],
      ["0", 1],
    ] as const) {
      const y = padY + plotH * frac;
      ctx.strokeStyle = ire === "50" ? "rgba(247,245,239,0.13)" : "rgba(247,245,239,0.06)";
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(padL, y);
      ctx.lineTo(w - padR, y);
      ctx.stroke();
      ctx.fillStyle = "rgba(247,245,239,0.34)";
      ctx.fillText(ire, padL - 4, y);
    }

    if (frame && frame.data.length > 0) {
      const { width: pw, height: ph, data } = frame;
      ctx.fillStyle = "rgba(52, 211, 153, 0.38)";
      for (let sx = 0; sx < pw; sx++) {
        const outX = padL + (sx / pw) * plotW;
        for (let sy = 0; sy < ph; sy += 2) {
          const idx = (sy * pw + sx) * 4;
          const r = data[idx] / 255;
          const g = data[idx + 1] / 255;
          const b = data[idx + 2] / 255;
          const luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
          const outY = padY + plotH * (1 - Math.max(0.01, Math.min(0.99, luma)));
          ctx.fillRect(outX, outY, 1.5, 1.5);
        }
      }
      return;
    }

    const activeGrade = gradeRef.current;
    const tSec = playheadRef.current;
    const liftOffset = (activeGrade.lift.r + activeGrade.lift.g + activeGrade.lift.b) / 3;
    const gainOffset = (activeGrade.gain.r + activeGrade.gain.g + activeGrade.gain.b) / 3;
    ctx.fillStyle = "rgba(52, 211, 153, 0.34)";
    for (let x = 0; x < plotW; x += 1.5) {
      const t = x / plotW;
      for (let s = 0; s < 6; s++) {
        const wave =
          Math.sin(t * 9.5 + tSec * 1.4 + s * 1.15) * 0.21 +
          Math.cos(t * 23 - tSec * 0.9 + s * 0.9) * 0.11;
        const luma =
          0.46 +
          wave * activeGrade.contrast +
          activeGrade.exposure * 0.15 +
          liftOffset * 0.25 +
          gainOffset * 0.35;
        const clamped = Math.max(0.04, Math.min(0.96, luma));
        const y = padY + plotH * (1 - clamped);
        ctx.fillRect(padL + x, y, 1.4, 1.4);
      }
    }
  }, []);

  const drawVectorscope = useCallback((frame: SampledPixelFrame | null) => {
    const canvas = vecRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const size = canvas.width;
    const cx = size / 2;
    const cy = size / 2;
    const r = size * 0.42;

    ctx.clearRect(0, 0, size, size);

    // Outer 100% & inner 50% concentric saturation rings
    ctx.strokeStyle = "rgba(247,245,239,0.14)";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.arc(cx, cy, r, 0, Math.PI * 2);
    ctx.stroke();

    ctx.strokeStyle = "rgba(247,245,239,0.07)";
    ctx.beginPath();
    ctx.arc(cx, cy, r * 0.5, 0, Math.PI * 2);
    ctx.stroke();

    // Crosshair axes
    ctx.strokeStyle = "rgba(247,245,239,0.08)";
    ctx.beginPath();
    ctx.moveTo(cx - r, cy);
    ctx.lineTo(cx + r, cy);
    ctx.moveTo(cx, cy - r);
    ctx.lineTo(cx, cy + r);
    ctx.stroke();

    // Rec.709 color target micro-labels around perimeter
    const targets: Array<[string, number, string]> = [
      ["R", 103, "rgba(251,113,133,0.65)"],
      ["Y", 167, "rgba(247,183,51,0.7)"],
      ["G", 241, "rgba(52,211,153,0.65)"],
      ["C", 283, "rgba(56,189,248,0.65)"],
      ["B", 347, "rgba(96,165,250,0.65)"],
      ["M", 61, "rgba(167,139,250,0.65)"],
    ];
    ctx.font = '600 8px "Sora Variable", "Sora", sans-serif';
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    for (const [label, deg, col] of targets) {
      const rad = (deg * Math.PI) / 180;
      const tx = cx + Math.cos(rad) * (r * 0.86);
      const ty = cy - Math.sin(rad) * (r * 0.86);
      ctx.fillStyle = col;
      ctx.fillText(label, tx, ty);
    }

    // Authentic SMPTE I-line / Skin-tone vector (~123 deg CCW from +Cb)
    const skinAngle = (123 * Math.PI) / 180;
    ctx.strokeStyle = "rgba(247,183,51,0.42)";
    ctx.beginPath();
    ctx.moveTo(cx, cy);
    ctx.lineTo(cx + Math.cos(skinAngle) * r, cy - Math.sin(skinAngle) * r);
    ctx.stroke();

    if (frame && frame.data.length > 0) {
      // Authentic Rec.709 Cb / Cr Vectorscope from graded WebGL2 pixels
      const { data } = frame;
      ctx.fillStyle = "rgba(247, 183, 51, 0.42)";
      for (let i = 0; i < data.length; i += 16) {
        const R = data[i] / 255;
        const G = data[i + 1] / 255;
        const B = data[i + 2] / 255;
        const cb = -0.114572 * R - 0.385428 * G + 0.5 * B;
        const cr = 0.5 * R - 0.454153 * G - 0.045847 * B;
        const px = cx + cb * r * 1.9;
        const py = cy - cr * r * 1.9;
        ctx.fillRect(px, py, 1.5, 1.5);
      }
      return;
    }

    const activeGrade = gradeRef.current;
    const tSec = playheadRef.current;
    const sat = Math.max(0.2, Math.min(2.0, activeGrade.saturation));
    ctx.fillStyle = "rgba(240, 90, 0, 0.42)";
    for (let i = 0; i < 150; i++) {
      const a = skinAngle + Math.sin(i * 1.7 + tSec) * 0.42 + activeGrade.tint * 0.4;
      const dist = ((i % 23) / 23) * r * 0.62 * sat;
      const px = cx + Math.cos(a) * dist + activeGrade.temp * 14;
      const py = cy - Math.sin(a) * dist - activeGrade.gain.r * 18;
      ctx.fillRect(px, py, 1.8, 1.8);
    }
  }, []);

  useImperativeHandle(
    scopesRef,
    () => ({
      drawPixelFrame: (frame: SampledPixelFrame | null) => {
        lastFrameRef.current = frame;
        const live = Boolean(frame && frame.data.length > 0);
        setHasLiveFrame((prev) => (prev === live ? prev : live));
        drawWaveform(frame);
        drawVectorscope(frame);
      },
    }),
    [drawWaveform, drawVectorscope],
  );

  // Only redraw synthetic fallback scopes on grade change when no live WebGL frame is active
  useEffect(() => {
    if (lastFrameRef.current && lastFrameRef.current.data.length > 0) return;
    drawWaveform(null);
    drawVectorscope(null);
  }, [grade, drawWaveform, drawVectorscope]);

  const lufsText =
    typeof integratedLufs === "number" ? `${integratedLufs.toFixed(1)} LUFS` : "-- LUFS";
  const clampedVoice = Math.max(0, Math.min(100, Math.round(voiceLevelPct)));
  const clampedMusic = Math.max(0, Math.min(100, Math.round(musicLevelPct)));

  return (
    <div
      data-cursor-anchor="scopes.panel"
      className="flex h-full min-h-0 flex-col gap-2 overflow-hidden rounded-xl border border-areia/[0.08] bg-carbono/65 p-2 shadow-[inset_0_1px_0_rgba(247,245,239,0.04)]"
    >
      <div className="flex min-h-0 flex-1 flex-col gap-1">
        <div className="flex shrink-0 items-center justify-between gap-1 whitespace-nowrap px-0.5 text-[9px] font-semibold uppercase tracking-[0.14em] text-areia/60">
          <span className="truncate">Waveform · Luma</span>
          <span
            className={`shrink-0 tabular-nums ${
              hasLiveFrame ? "text-amarelo-ideias" : "text-areia/45"
            }`}
            title={hasLiveFrame ? "Live WebGL2 graded pixel feed" : "0–100 IRE synthetic reference"}
          >
            {hasLiveFrame ? "LIVE" : "IRE"}
          </span>
        </div>
        <div className="min-h-0 flex-1 overflow-hidden rounded-lg border border-areia/[0.08] bg-void/95">
          <canvas
            ref={waveRef}
            width={240}
            height={110}
            className="h-full w-full"
          />
        </div>
      </div>

      <div className="flex min-h-0 flex-1 flex-col gap-1">
        <div className="flex shrink-0 items-center justify-between gap-1 whitespace-nowrap px-0.5 text-[9px] font-semibold uppercase tracking-[0.14em] text-areia/60">
          <span className="truncate">Vectorscope</span>
          <span
            className="shrink-0 text-amarelo-ideias/80"
            title="Rec.709 Cb/Cr with 123° Skin-Tone I-Line"
          >
            Rec.709
          </span>
        </div>
        <div className="flex min-h-0 flex-1 items-center justify-center overflow-hidden rounded-lg border border-areia/[0.08] bg-void/95 p-1">
          <canvas
            ref={vecRef}
            width={152}
            height={152}
            className="aspect-square h-full max-h-[144px] w-auto max-w-full"
          />
        </div>
      </div>

      <div
        data-cursor-anchor="audio.mixer"
        className="shrink-0 space-y-1.5 rounded-lg border border-areia/[0.08] bg-void/85 p-2"
      >
        <div className="flex items-center justify-between text-[9px] font-semibold uppercase tracking-[0.14em] text-areia/60">
          <span>Loudness</span>
          <span className="tabular-nums text-amarelo-ideias">{lufsText}</span>
        </div>
        <div className="space-y-1.5 text-[9px] text-areia/70">
          <div className="flex items-center gap-2">
            <span className="w-9 font-medium tracking-wider text-areia/55">VOICE</span>
            <div className="relative h-1.5 flex-1 overflow-hidden rounded-full bg-grafite">
              <div
                className="h-full origin-left rounded-full bg-track-a1 transition-transform duration-150 motion-reduce:transition-none"
                style={{ transform: `scaleX(${clampedVoice / 100})` }}
              />
              <span
                aria-hidden="true"
                className="pointer-events-none absolute top-0 bottom-0 left-[61.1%] w-px bg-amarelo-ideias/50"
                title="-14 LUFS target"
              />
            </div>
          </div>
          <div className="flex items-center gap-2">
            <span className="w-9 font-medium tracking-wider text-areia/55">MUSIC</span>
            <div className="relative h-1.5 flex-1 overflow-hidden rounded-full bg-grafite">
              <div
                className="h-full origin-left rounded-full bg-track-a2 transition-transform duration-150 motion-reduce:transition-none"
                style={{ transform: `scaleX(${clampedMusic / 100})` }}
              />
              <span
                aria-hidden="true"
                className="pointer-events-none absolute top-0 bottom-0 left-[61.1%] w-px bg-amarelo-ideias/50"
                title="-14 LUFS target"
              />
            </div>
          </div>
        </div>
        <div className="relative ml-11 h-2.5 tabular-nums text-[8px] text-areia/45">
          <span className="absolute left-0">-36</span>
          <span className="absolute left-[33.3%] -translate-x-1/2">-24</span>
          <span className="absolute left-[61.1%] -translate-x-1/2 text-amarelo-ideias/75">-14</span>
          <span className="absolute left-[83.3%] -translate-x-1/2">-6</span>
          <span className="absolute right-0">0dB</span>
        </div>
      </div>
    </div>
  );
}
