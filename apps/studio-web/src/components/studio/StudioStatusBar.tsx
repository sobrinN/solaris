import { formatCompactDuration } from "../../lib/timelineFormat";

interface StudioStatusBarProps {
  wsConnected: boolean;
  revision: number;
  clipCount: number;
  totalDurationSec: number;
  aspectBadge: string;
  width: number;
  height: number;
  fps: number;
}

export function StudioStatusBar({
  wsConnected,
  revision,
  clipCount,
  totalDurationSec,
  aspectBadge,
  width,
  height,
  fps,
}: StudioStatusBarProps) {
  return (
    <footer className="flex h-6 shrink-0 items-center justify-between gap-3 border-t border-areia/[0.08] bg-carbono/90 px-3 text-[10px] text-areia/55">
      <div className="flex min-w-0 items-center gap-2.5">
        <span className="flex items-center gap-1.5">
          <span
            className={`h-1.5 w-1.5 rounded-full ${
              wsConnected ? "bg-track-a1" : "bg-vermelho-foco"
            }`}
          />
          <span className={wsConnected ? "text-areia/75" : "text-areia"}>
            {wsConnected ? "Engine Live" : "Reconnecting"}
          </span>
        </span>
        <span className="text-areia/20">·</span>
        <span className="tabular-nums text-areia/65">r{revision}</span>
        <span className="text-areia/20">·</span>
        <span className="tabular-nums">
          {clipCount} {clipCount === 1 ? "clip" : "clips"}
        </span>
        <span className="text-areia/20">·</span>
        <span className="tabular-nums text-amarelo-ideias/85">
          {formatCompactDuration(totalDurationSec)}
        </span>
      </div>

      <div className="hidden items-center gap-3 text-[10px] text-areia/40 xl:flex">
        <span>Space Play</span>
        <span>·</span>
        <span>S Split</span>
        <span>·</span>
        <span>⌘Z Undo</span>
        <span>·</span>
        <span>Drag Marquee</span>
      </div>

      <div className="flex shrink-0 items-center gap-2 tabular-nums text-areia/65">
        <span>{aspectBadge}</span>
        <span className="text-areia/25">·</span>
        <span>
          {width}×{height}
        </span>
        <span className="text-areia/25">·</span>
        <span>{Math.round(fps)} FPS</span>
      </div>
    </footer>
  );
}
