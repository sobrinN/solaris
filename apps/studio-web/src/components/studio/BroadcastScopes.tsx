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
    ctx.clearRect(0, 0, w, h);

    ctx.strokeStyle = "rgba(247,245,239,0.08)";
    ctx.lineWidth = 1;
    for (const frac of [0.2, 0.4, 0.6, 0.8]) {
      ctx.beginPath();
      ctx.moveTo(0, h * frac);
      ctx.lineTo(w, h * frac);
      ctx.stroke();
    }

    if (frame && frame.data.length > 0) {
      const { width: pw, height: ph, data } = frame;
      ctx.fillStyle = "rgba(52, 211, 153, 0.28)";
      for (let sx = 0; sx < pw; sx++) {
        const outX = (sx / pw) * w;
        for (let sy = 0; sy < ph; sy += 2) {
          const idx = (sy * pw + sx) * 4;
          const r = data[idx] / 255;
          const g = data[idx + 1] / 255;
          const b = data[idx + 2] / 255;
          const luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
          const outY = h * (1 - Math.max(0.02, Math.min(0.98, luma)));
          ctx.fillRect(outX, outY, 1.5, 1.5);
        }
      }
      return;
    }

    const activeGrade = gradeRef.current;
    const tSec = playheadRef.current;
    const liftOffset = (activeGrade.lift.r + activeGrade.lift.g + activeGrade.lift.b) / 3;
    const gainOffset = (activeGrade.gain.r + activeGrade.gain.g + activeGrade.gain.b) / 3;
    ctx.fillStyle = "rgba(52, 211, 153, 0.24)";
    for (let x = 0; x < w; x += 2) {
      const t = x / w;
      for (let s = 0; s < 5; s++) {
        const wave =
          Math.sin(t * 9.5 + tSec * 1.4 + s * 1.3) * 0.22 +
          Math.cos(t * 21 - tSec * 0.9 + s) * 0.12;
        const luma =
          0.45 +
          wave * activeGrade.contrast +
          activeGrade.exposure * 0.15 +
          liftOffset * 0.25 +
          gainOffset * 0.35;
        const clamped = Math.max(0.05, Math.min(0.95, luma));
        const y = h * (1 - clamped);
        ctx.fillRect(x, y, 1.5, 1.5);
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

    ctx.strokeStyle = "rgba(247,245,239,0.12)";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.arc(cx, cy, r, 0, Math.PI * 2);
    ctx.stroke();

    // Authentic SMPTE I-line / Skin-tone vector (~123 deg CCW from +Cb: negative Cb, positive Cr)
    const skinAngle = (123 * Math.PI) / 180;
    ctx.strokeStyle = "rgba(247,183,51,0.34)";
    ctx.beginPath();
    ctx.moveTo(cx, cy);
    ctx.lineTo(
      cx + Math.cos(skinAngle) * r,
      cy - Math.sin(skinAngle) * r,
    );
    ctx.stroke();

    if (frame && frame.data.length > 0) {
      // Authentic Rec.709 Cb / Cr Vectorscope from graded WebGL2 pixels
      const { data } = frame;
      ctx.fillStyle = "rgba(240, 90, 0, 0.35)";
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
    ctx.fillStyle = "rgba(240, 90, 0, 0.32)";
    for (let i = 0; i < 120; i++) {
      const a = skinAngle + Math.sin(i * 1.7 + tSec) * 0.35 + activeGrade.tint * 0.4;
      const dist = ((i % 19) / 19) * r * 0.55 * sat;
      const px = cx + Math.cos(a) * dist + activeGrade.temp * 14;
      const py = cy - Math.sin(a) * dist - activeGrade.gain.r * 18;
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
      className="flex h-full flex-col gap-3 rounded-2xl border border-areia/10 bg-carbono/70 p-3"
    >
      <div className="flex flex-col gap-1.5">
        <div className="flex items-center justify-between text-[10px] font-semibold uppercase tracking-[0.14em] text-areia/60">
          <span>Waveform · Luma</span>
          <span
            className={`tabular-nums ${
              hasLiveFrame ? "text-amarelo-ideias" : "text-areia/45"
            }`}
          >
            {hasLiveFrame ? "LIVE" : "0–100 IRE"}
          </span>
        </div>
        <canvas
          ref={waveRef}
          width={240}
          height={96}
          className="w-full rounded-xl border border-areia/10 bg-void/90"
        />
      </div>

      <div className="flex flex-col gap-1.5">
        <div className="flex items-center justify-between text-[10px] font-semibold uppercase tracking-[0.14em] text-areia/60">
          <span>Vectorscope</span>
          <span className="text-amarelo-ideias/80">Rec.709 · Skin</span>
        </div>
        <div className="grid place-items-center rounded-xl border border-areia/10 bg-void/90 py-2">
          <canvas ref={vecRef} width={132} height={132} className="h-[124px] w-[124px]" />
        </div>
      </div>

      <div
        data-cursor-anchor="audio.mixer"
        className="mt-auto space-y-2 rounded-xl border border-areia/10 bg-void/75 p-2.5"
      >
        <div className="flex items-center justify-between text-[10px] font-semibold uppercase tracking-[0.14em] text-areia/60">
          <span>Loudness</span>
          <span className="tabular-nums text-amarelo-ideias">{lufsText}</span>
        </div>
        <div className="space-y-1.5 text-[10px] text-areia/70">
          <div className="flex items-center gap-2">
            <span className="w-10">VOICE</span>
            <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-grafite">
              <div
                className="h-full origin-left rounded-full bg-track-a1 transition-transform duration-150 motion-reduce:transition-none"
                style={{ transform: `scaleX(${clampedVoice / 100})` }}
              />
            </div>
          </div>
          <div className="flex items-center gap-2">
            <span className="w-10">MUSIC</span>
            <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-grafite">
              <div
                className="h-full origin-left rounded-full bg-track-a2 transition-transform duration-150 motion-reduce:transition-none"
                style={{ transform: `scaleX(${clampedMusic / 100})` }}
              />
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
