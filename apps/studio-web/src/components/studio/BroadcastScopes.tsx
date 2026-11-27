import { useCallback, useEffect, useImperativeHandle, useRef, useState, type Ref } from "react";
import { Activity } from "lucide-react";
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
    const padL = 26;
    const padR = 6;
    const padY = 8;
    const plotW = w - padL - padR;
    const plotH = h - padY * 2;
    ctx.clearRect(0, 0, w, h);

    // Broadcast IRE graticule (100, 75, 50, 25, 0)
    ctx.font = '600 9px "Sora Variable", "Sora", sans-serif';
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
      ctx.strokeStyle = ire === "50" ? "rgba(247,245,239,0.16)" : "rgba(247,245,239,0.065)";
      ctx.lineWidth = 1;
      if (ire === "50") ctx.setLineDash([3, 3]);
      ctx.beginPath();
      ctx.moveTo(padL, y);
      ctx.lineTo(w - padR, y);
      ctx.stroke();
      if (ire === "50") ctx.setLineDash([]);
      ctx.fillStyle = "rgba(247,245,239,0.38)";
      ctx.fillText(ire, padL - 5, y);
    }

    if (frame && frame.data.length > 0) {
      const { width: pw, height: ph, data } = frame;
      for (let sx = 0; sx < pw; sx++) {
        const outX = padL + (sx / pw) * plotW;
        for (let sy = 0; sy < ph; sy += 2) {
          const idx = (sy * pw + sx) * 4;
          const r = data[idx] / 255;
          const g = data[idx + 1] / 255;
          const b = data[idx + 2] / 255;
          const luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
          const clamped = Math.max(0.01, Math.min(0.99, luma));
          const outY = padY + plotH * (1 - clamped);
          ctx.fillStyle =
            clamped > 0.88
              ? "rgba(247, 183, 51, 0.55)"
              : "rgba(52, 211, 153, 0.42)";
          ctx.fillRect(outX, outY, 1.8, 1.8);
        }
      }
      return;
    }

    // Rich multi-harmonic phosphor distribution cloud when paused without live WebGL video frames
    const activeGrade = gradeRef.current;
    const tSec = playheadRef.current;
    const liftOffset = (activeGrade.lift.r + activeGrade.lift.g + activeGrade.lift.b) / 3;
    const gainOffset = (activeGrade.gain.r + activeGrade.gain.g + activeGrade.gain.b) / 3;
    for (let x = 0; x < plotW; x += 2) {
      const t = x / plotW;
      const envelope = 0.5 + 0.45 * Math.sin(t * Math.PI);
      for (let s = 0; s < 10; s++) {
        const harmonic =
          Math.sin(t * 7.2 + tSec * 0.9 + s * 0.85) * 0.16 +
          Math.cos(t * 19.5 - tSec * 0.6 + s * 1.4) * 0.09 +
          Math.sin(t * 37.0 + s * 2.1) * 0.04;
        const scatter = ((s % 3) - 1) * 0.05 * envelope;
        const luma =
          0.44 +
          (harmonic + scatter) * activeGrade.contrast +
          activeGrade.exposure * 0.18 +
          liftOffset * 0.25 +
          gainOffset * 0.35;
        const clamped = Math.max(0.03, Math.min(0.97, luma));
        const y = padY + plotH * (1 - clamped);
        ctx.fillStyle =
          clamped > 0.85
            ? "rgba(247, 183, 51, 0.36)"
            : s % 2 === 0
              ? "rgba(52, 211, 153, 0.34)"
              : "rgba(56, 189, 248, 0.16)";
        ctx.fillRect(padL + x, y, 1.5, 1.5);
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

    // Concentric 100%, 75%, 50%, 25% saturation rings
    for (const [frac, alpha] of [
      [1, 0.16],
      [0.75, 0.06],
      [0.5, 0.09],
      [0.25, 0.05],
    ] as const) {
      ctx.strokeStyle = `rgba(247,245,239,${alpha})`;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.arc(cx, cy, r * frac, 0, Math.PI * 2);
      ctx.stroke();
    }

    // Outer 30-degree degree ticks
    ctx.strokeStyle = "rgba(247,245,239,0.16)";
    for (let deg = 0; deg < 360; deg += 30) {
      const rad = (deg * Math.PI) / 180;
      ctx.beginPath();
      ctx.moveTo(cx + Math.cos(rad) * (r * 0.96), cy - Math.sin(rad) * (r * 0.96));
      ctx.lineTo(cx + Math.cos(rad) * r, cy - Math.sin(rad) * r);
      ctx.stroke();
    }

    // Crosshair axes
    ctx.strokeStyle = "rgba(247,245,239,0.08)";
    ctx.beginPath();
    ctx.moveTo(cx - r, cy);
    ctx.lineTo(cx + r, cy);
    ctx.moveTo(cx, cy - r);
    ctx.lineTo(cx, cy + r);
    ctx.stroke();

    // Rec.709 color target reticles & micro-labels around perimeter
    const targets: Array<[string, number, string]> = [
      ["R", 103, "rgba(251,113,133,0.75)"],
      ["Y", 167, "rgba(247,183,51,0.8)"],
      ["G", 241, "rgba(52,211,153,0.75)"],
      ["C", 283, "rgba(56,189,248,0.75)"],
      ["B", 347, "rgba(96,165,250,0.75)"],
      ["M", 61, "rgba(167,139,250,0.75)"],
    ];
    ctx.font = '600 9px "Sora Variable", "Sora", sans-serif';
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    for (const [label, deg, col] of targets) {
      const rad = (deg * Math.PI) / 180;
      const tx = cx + Math.cos(rad) * (r * 0.86);
      const ty = cy - Math.sin(rad) * (r * 0.86);
      ctx.strokeStyle = col;
      ctx.strokeRect(tx - 6, ty - 6, 12, 12);
      ctx.fillStyle = col;
      ctx.fillText(label, tx, ty);
    }

    // Authentic SMPTE I-line / Skin-tone vector (~123 deg CCW from +Cb)
    const skinAngle = (123 * Math.PI) / 180;
    ctx.strokeStyle = "rgba(247,183,51,0.48)";
    ctx.beginPath();
    ctx.moveTo(cx, cy);
    ctx.lineTo(cx + Math.cos(skinAngle) * r, cy - Math.sin(skinAngle) * r);
    ctx.stroke();

    if (frame && frame.data.length > 0) {
      const { data } = frame;
      ctx.fillStyle = "rgba(247, 183, 51, 0.45)";
      for (let i = 0; i < data.length; i += 16) {
        const R = data[i] / 255;
        const G = data[i + 1] / 255;
        const B = data[i + 2] / 255;
        const cb = -0.114572 * R - 0.385428 * G + 0.5 * B;
        const cr = 0.5 * R - 0.454153 * G - 0.045847 * B;
        const px = cx + cb * r * 1.9;
        const py = cy - cr * r * 1.9;
        ctx.fillRect(px, py, 1.8, 1.8);
      }
      return;
    }

    const activeGrade = gradeRef.current;
    const tSec = playheadRef.current;
    const sat = Math.max(0.2, Math.min(2.0, activeGrade.saturation));
    ctx.fillStyle = "rgba(240, 90, 0, 0.48)";
    for (let i = 0; i < 190; i++) {
      const a = skinAngle + Math.sin(i * 1.7 + tSec) * 0.42 + activeGrade.tint * 0.4;
      const dist = ((i % 27) / 27) * r * 0.62 * sat;
      const px = cx + Math.cos(a) * dist + activeGrade.temp * 16;
      const py = cy - Math.sin(a) * dist - activeGrade.gain.r * 20;
      ctx.fillRect(px, py, 2, 2);
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
      className="solaris-panel flex h-full min-h-0 flex-col overflow-hidden rounded-xl"
    >
      {/* Unified h-9 Header */}
      <div className="flex h-9 shrink-0 items-center justify-between gap-2 border-b border-areia/[0.08] px-2.5">
        <div className="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-[0.14em] text-areia/75">
          <Activity className="h-3 w-3 text-amarelo-ideias" />
          <span>Scopes</span>
        </div>
        <span
          className={`tabular-nums rounded-full border px-2 py-0.5 text-[9px] font-semibold uppercase tracking-wider ${
            hasLiveFrame
              ? "border-amarelo-ideias/45 bg-amarelo-ideias/12 text-amarelo-ideias"
              : "border-areia/12 bg-void/70 text-areia/55"
          }`}
          title={hasLiveFrame ? "Live WebGL2 graded pixel feed" : "Rec.709 / 0–100 IRE reference"}
        >
          {hasLiveFrame ? "LIVE" : "REC.709"}
        </span>
      </div>

      <div className="flex min-h-0 flex-1 flex-col gap-2 p-2">
        <div className="flex min-h-0 flex-1 flex-col gap-1">
          <div className="flex shrink-0 items-center justify-between gap-1 whitespace-nowrap px-0.5 text-[9px] font-semibold uppercase tracking-[0.14em] text-areia/55">
            <span className="truncate">Waveform · Luma</span>
            <span className="tabular-nums text-areia/40">IRE</span>
          </div>
          <div className="min-h-0 flex-1 overflow-hidden rounded-lg border border-areia/[0.08] bg-void/95">
            <canvas
              ref={waveRef}
              width={320}
              height={150}
              className="h-full w-full"
            />
          </div>
        </div>

        <div className="flex min-h-0 flex-1 flex-col gap-1">
          <div className="flex shrink-0 items-center justify-between gap-1 whitespace-nowrap px-0.5 text-[9px] font-semibold uppercase tracking-[0.14em] text-areia/55">
            <span className="truncate">Vectorscope</span>
            <span
              className="shrink-0 text-amarelo-ideias/80"
              title="Rec.709 Cb/Cr with 123° Skin-Tone I-Line"
            >
              123° Skin
            </span>
          </div>
          <div className="flex min-h-0 flex-1 items-center justify-center overflow-hidden rounded-lg border border-areia/[0.08] bg-void/95 p-1">
            <canvas
              ref={vecRef}
              width={200}
              height={200}
              className="aspect-square h-full max-h-[152px] w-auto max-w-full"
            />
          </div>
        </div>

        <div
          data-cursor-anchor="audio.mixer"
          className="shrink-0 space-y-1.5 rounded-lg border border-areia/[0.08] bg-void/85 p-2"
        >
          <div className="flex items-center justify-between text-[9px] font-semibold uppercase tracking-[0.14em] text-areia/60">
            <span>Loudness</span>
            <span className="tabular-nums rounded-full border border-amarelo-ideias/30 bg-amarelo-ideias/10 px-1.5 py-0.5 text-[9px] font-semibold text-amarelo-ideias">
              {lufsText}
            </span>
          </div>
          <div className="space-y-1.5 text-[9px] text-areia/70">
            <div className="flex items-center gap-2">
              <span className="w-9 font-medium tracking-wider text-areia/55">VOICE</span>
              <div className="relative h-2 flex-1 overflow-hidden rounded-full bg-grafite">
                <div
                  className="h-full origin-left rounded-full bg-track-a1 transition-transform duration-150 motion-reduce:transition-none"
                  style={{ transform: `scaleX(${clampedVoice / 100})` }}
                />
                <span
                  aria-hidden="true"
                  className="pointer-events-none absolute top-0 bottom-0 left-[61.1%] w-[1.5px] bg-amarelo-ideias/75 shadow-[0_0_6px_rgba(247,183,51,0.75)]"
                  title="-14 LUFS target"
                />
              </div>
            </div>
            <div className="flex items-center gap-2">
              <span className="w-9 font-medium tracking-wider text-areia/55">MUSIC</span>
              <div className="relative h-2 flex-1 overflow-hidden rounded-full bg-grafite">
                <div
                  className="h-full origin-left rounded-full bg-track-a2 transition-transform duration-150 motion-reduce:transition-none"
                  style={{ transform: `scaleX(${clampedMusic / 100})` }}
                />
                <span
                  aria-hidden="true"
                  className="pointer-events-none absolute top-0 bottom-0 left-[61.1%] w-[1.5px] bg-amarelo-ideias/75 shadow-[0_0_6px_rgba(247,183,51,0.75)]"
                  title="-14 LUFS target"
                />
              </div>
            </div>
          </div>
          <div className="relative ml-11 h-2.5 tabular-nums text-[8px] text-areia/45">
            <span className="absolute left-0">-36</span>
            <span className="absolute left-[33.3%] -translate-x-1/2">-24</span>
            <span className="absolute left-[61.1%] -translate-x-1/2 font-semibold text-amarelo-ideias/85">-14</span>
            <span className="absolute left-[83.3%] -translate-x-1/2">-6</span>
            <span className="absolute right-0">0dB</span>
          </div>
        </div>
      </div>
    </div>
  );
}
