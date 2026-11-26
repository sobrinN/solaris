import { useEffect, useRef } from "react";
import type { RgbTriplet } from "@solaris/engine-client";

interface ColorWheelControlProps {
  label: string;
  value: RgbTriplet;
  onChange: (next: RgbTriplet) => void;
  onCommit?: (finalVal: RgbTriplet) => void;
}

export function ColorWheelControl({
  label,
  value,
  onChange,
  onCommit,
}: ColorWheelControlProps) {
  const wheelRef = useRef<HTMLDivElement | null>(null);
  const latestRef = useRef<RgbTriplet>(value);
  const draggingRef = useRef(false);
  const draggedRef = useRef(false);
  const keyCommitTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  latestRef.current = value;

  useEffect(() => {
    return () => {
      if (keyCommitTimerRef.current) clearTimeout(keyCommitTimerRef.current);
    };
  }, []);

  const scheduleCommit = (next: RgbTriplet) => {
    if (keyCommitTimerRef.current) clearTimeout(keyCommitTimerRef.current);
    keyCommitTimerRef.current = setTimeout(() => {
      keyCommitTimerRef.current = null;
      onCommit?.(next);
    }, 220);
  };

  const updateFromPointer = (clientX: number, clientY: number) => {
    const el = wheelRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const cx = rect.left + rect.width / 2;
    const cy = rect.top + rect.height / 2;
    const radius = rect.width / 2;
    let dx = (clientX - cx) / radius;
    let dy = (clientY - cy) / radius;
    const mag = Math.hypot(dx, dy);
    if (mag > 1) {
      dx /= mag;
      dy /= mag;
    }

    // Map 2D wheel offset (dx, dy) into balanced RGB offset in [-0.35, +0.35]
    const angle = Math.atan2(dy, dx);
    const strength = Math.min(1, mag) * 0.35;
    const r = Math.cos(angle) * strength;
    const g = Math.cos(angle - (2 * Math.PI) / 3) * strength;
    const b = Math.cos(angle + (2 * Math.PI) / 3) * strength;
    const next = {
      r: Number(r.toFixed(3)),
      g: Number(g.toFixed(3)),
      b: Number(b.toFixed(3)),
    };
    latestRef.current = next;
    onChange(next);
  };

  // Inverse projection for handle dot position (linearly proportional to magnitude)
  const px = (value.r - 0.5 * value.g - 0.5 * value.b) * 1.9;
  const py = ((Math.sqrt(3) / 2) * (value.g - value.b)) * 1.9;
  const mag = Math.hypot(px, py);
  const norm = mag > 1 ? 1 / mag : 1;
  const dotX = px * norm * 21;
  const dotY = py * norm * 21;
  const pctValue = Math.round(Math.min(1, mag) * 100);

  return (
    <div className="flex flex-col items-center gap-1">
      <div
        ref={wheelRef}
        role="slider"
        tabIndex={0}
        aria-label={`${label} color wheel`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={pctValue}
        aria-valuetext={`R ${value.r.toFixed(2)}, G ${value.g.toFixed(2)}, B ${value.b.toFixed(2)}`}
        onDoubleClick={(e) => {
          e.stopPropagation();
          if (keyCommitTimerRef.current) {
            clearTimeout(keyCommitTimerRef.current);
            keyCommitTimerRef.current = null;
          }
          draggingRef.current = false;
          draggedRef.current = false;
          const reset = { r: 0, g: 0, b: 0 };
          latestRef.current = reset;
          onChange(reset);
          onCommit?.(reset);
        }}
        onKeyDown={(e) => {
          const step = 0.03;
          let next: RgbTriplet | null = null;
          if (e.key === "0" || e.key === "Backspace") {
            next = { r: 0, g: 0, b: 0 };
          } else if (e.key === "ArrowRight") {
            next = {
              r: Number(Math.min(0.35, value.r + step).toFixed(3)),
              g: Number(Math.max(-0.35, value.g - step * 0.5).toFixed(3)),
              b: Number(Math.max(-0.35, value.b - step * 0.5).toFixed(3)),
            };
          } else if (e.key === "ArrowLeft") {
            next = {
              r: Number(Math.max(-0.35, value.r - step).toFixed(3)),
              g: Number(Math.min(0.35, value.g + step * 0.5).toFixed(3)),
              b: Number(Math.min(0.35, value.b + step * 0.5).toFixed(3)),
            };
          } else if (e.key === "ArrowUp") {
            next = {
              r: value.r,
              g: Number(Math.max(-0.35, value.g - step).toFixed(3)),
              b: Number(Math.min(0.35, value.b + step).toFixed(3)),
            };
          } else if (e.key === "ArrowDown") {
            next = {
              r: value.r,
              g: Number(Math.min(0.35, value.g + step).toFixed(3)),
              b: Number(Math.max(-0.35, value.b - step).toFixed(3)),
            };
          }
          if (next) {
            e.preventDefault();
            e.stopPropagation();
            latestRef.current = next;
            onChange(next);
            if (e.key === "0" || e.key === "Backspace") {
              if (keyCommitTimerRef.current) {
                clearTimeout(keyCommitTimerRef.current);
                keyCommitTimerRef.current = null;
              }
              onCommit?.(next);
            } else {
              scheduleCommit(next);
            }
          }
        }}
        onPointerDown={(e) => {
          if (e.button !== 0) return;
          if (e.detail >= 2) return; // Avoid emitting drag ops before double-click reset
          draggingRef.current = true;
          draggedRef.current = false;
          e.currentTarget.setPointerCapture(e.pointerId);
          updateFromPointer(e.clientX, e.clientY);
          draggedRef.current = true;
        }}
        onPointerMove={(e) => {
          if (draggingRef.current && e.buttons === 1) {
            draggedRef.current = true;
            updateFromPointer(e.clientX, e.clientY);
          }
        }}
        onPointerUp={(e) => {
          if (!draggingRef.current) return;
          draggingRef.current = false;
          if (e.currentTarget.hasPointerCapture(e.pointerId)) {
            e.currentTarget.releasePointerCapture(e.pointerId);
          }
          if (draggedRef.current) {
            draggedRef.current = false;
            onCommit?.(latestRef.current);
          }
        }}
        onPointerCancel={() => {
          draggingRef.current = false;
          draggedRef.current = false;
        }}
        title="Drag to grade · Double-click to reset"
        className="relative h-14 w-14 cursor-crosshair select-none rounded-full border border-areia/15 shadow-[inset_0_0_12px_rgba(0,0,0,0.85)] focus-visible:outline-2 focus-visible:outline-laranja-criacao xl:h-[62px] xl:w-[62px]"
        style={{
          background:
            "conic-gradient(from 90deg, #ff4d4d 0deg, #f7b733 60deg, #34d399 120deg, #38bdf8 180deg, #60a5fa 240deg, #a78bfa 300deg, #ff4d4d 360deg)",
        }}
      >
        <div className="pointer-events-none absolute inset-[5px] rounded-full border border-areia/10 bg-void/88" />
        <div className="pointer-events-none absolute top-1/2 right-2.5 left-2.5 h-px -translate-y-1/2 bg-areia/10" />
        <div className="pointer-events-none absolute top-2.5 bottom-2.5 left-1/2 w-px -translate-x-1/2 bg-areia/10" />
        <div
          className="pointer-events-none absolute top-1/2 left-1/2 h-2.5 w-2.5 rounded-full border border-areia bg-laranja-criacao shadow-[0_0_8px_rgba(240,90,0,0.85)]"
          style={{
            transform: `translate(calc(-50% + ${dotX.toFixed(1)}px), calc(-50% + ${dotY.toFixed(1)}px))`,
          }}
        />
      </div>
      <span className="text-[9px] font-semibold tracking-[0.14em] text-areia/65">
        {label}
      </span>
    </div>
  );
}
