import { useEffect, useState } from "react";
import type { AgentRole } from "@solaris/engine-client";

export interface ActiveCursorState {
  agentId: string;
  role: AgentRole;
  anchor: string;
  label: string;
  color: string;
  x: number;
  y: number;
  lastUpdated: number;
}

const ROLE_COLORS: Record<AgentRole, string> = {
  user: "#F7F5EF",
  director: "#F7B733",
  razor: "#60A5FA",
  b_roll: "#38BDF8",
  grade: "#F05A00",
  caps: "#F7B733",
  gfx: "#FB7185",
  mix: "#34D399",
  mcp_external: "#A78BFA",
};

const CHOREOGRAPHY_ACTORS: Array<{
  agentId: string;
  role: AgentRole;
  anchors: string[];
  xBias: number;
  label: string;
}> = [
  { agentId: "razor-01", role: "razor", anchors: ["timeline.track.v1", "source.monitor"], xBias: 0.22, label: "trimming breath" },
  { agentId: "caps-01", role: "caps", anchors: ["captions.inspector", "timeline.track.t1", "program.stage"], xBias: 0.38, label: "safe-zone check" },
  { agentId: "grade-01", role: "grade", anchors: ["color.inspector", "scopes.panel"], xBias: 0.65, label: "balancing gain" },
  { agentId: "gfx-01", role: "gfx", anchors: ["effects.library", "timeline.track.g1"], xBias: 0.52, label: "easing card" },
  { agentId: "broll-01", role: "b_roll", anchors: ["media.pool", "timeline.track.v2"], xBias: 0.72, label: "syncing insert" },
  { agentId: "mix-01", role: "mix", anchors: ["timeline.track.a2", "audio.mixer"], xBias: 0.82, label: "ducking -18 dB" },
];

function escapeAttr(val: string): string {
  if (typeof CSS !== "undefined" && typeof CSS.escape === "function") {
    return CSS.escape(val);
  }
  return val.replace(/["\\]/g, "\\$&");
}

function resolveAnchorCoordinates(
  anchor: string,
  xBias = 0.5,
): { x: number; y: number } | null {
  const prefix = anchor.split(".").slice(0, 2).join(".");
  const el =
    document.querySelector<HTMLElement>(`[data-cursor-anchor="${escapeAttr(anchor)}"]`) ??
    document.querySelector<HTMLElement>(`[data-cursor-anchor="${escapeAttr(prefix)}"]`) ??
    document.querySelector<HTMLElement>(`[data-cursor-anchor="timeline"]`);
  if (!el) return null;
  const rect = el.getBoundingClientRect();
  if (rect.width === 0 || rect.height === 0) return null;

  const spreadX = rect.width * (0.15 + xBias * 0.65);
  const jitterX = (Math.random() - 0.5) * Math.min(rect.width * 0.18, 70);
  const jitterY = (Math.random() - 0.5) * Math.min(rect.height * 0.45, 28);
  const minX = rect.left + 8;
  const maxX = Math.max(minX, rect.right - 90);
  return {
    x: Math.max(minX, Math.min(maxX, rect.left + spreadX + jitterX)),
    y: rect.top + rect.height * 0.5 + jitterY,
  };
}

interface SwarmCursorLayerProps {
  enabled: boolean;
  demoMode?: boolean;
  swarmActive?: boolean;
  activeRoles?: AgentRole[];
  latestRealOp?: { agentId: string; role: AgentRole; anchor: string; summary: string } | null;
}

const CURSOR_IDLE_TTL_MS = 6500;

export function SwarmCursorLayer({
  enabled,
  demoMode = false,
  swarmActive = false,
  activeRoles = [],
  latestRealOp,
}: SwarmCursorLayerProps) {
  const [cursors, setCursors] = useState<Record<string, ActiveCursorState>>({});

  useEffect(() => {
    if (!enabled || !latestRealOp || latestRealOp.role === "user") return;
    const actorBias =
      CHOREOGRAPHY_ACTORS.find((a) => a.role === latestRealOp.role)?.xBias ?? 0.5;
    const coords = resolveAnchorCoordinates(latestRealOp.anchor, actorBias);
    if (!coords) return;
    setCursors((prev) => ({
      ...prev,
      [latestRealOp.agentId]: {
        agentId: latestRealOp.agentId,
        role: latestRealOp.role,
        anchor: latestRealOp.anchor,
        label: latestRealOp.summary.slice(0, 28),
        color: ROLE_COLORS[latestRealOp.role] ?? "#F05A00",
        x: coords.x,
        y: coords.y,
        lastUpdated: Date.now(),
      },
    }));
  }, [enabled, latestRealOp]);

  const activeRolesKey = activeRoles.join(",");

  // Subtle choreographed scanning micro-motions during demo mode OR active real Swarm runs (LLM thinking gaps)
  useEffect(() => {
    if (!enabled || (!demoMode && !swarmActive)) return;
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (reduced) return;

    const roleList = activeRolesKey ? (activeRolesKey.split(",") as AgentRole[]) : [];

    const tick = () => {
      const now = Date.now();
      const targetActors =
        swarmActive && roleList.length > 0
          ? CHOREOGRAPHY_ACTORS.filter((a) => roleList.includes(a.role))
          : swarmActive
            ? [
                {
                  agentId: "director-01",
                  role: "director" as AgentRole,
                  anchors: ["media.pool", "program.stage", "timeline"],
                  xBias: 0.45,
                  label: "directing cut",
                },
              ]
            : CHOREOGRAPHY_ACTORS;

      setCursors((prev) => {
        const next = { ...prev };
        for (const actor of targetActors) {
          const existing = next[actor.agentId];
          if (existing && now - existing.lastUpdated < 2100) continue;
          const anchor = actor.anchors[Math.floor(Math.random() * actor.anchors.length)];
          const coords = resolveAnchorCoordinates(anchor, actor.xBias);
          if (!coords) continue;
          next[actor.agentId] = {
            agentId: actor.agentId,
            role: actor.role,
            anchor,
            label: existing?.label ?? actor.label,
            color: ROLE_COLORS[actor.role],
            x: coords.x,
            y: coords.y,
            lastUpdated: now,
          };
        }
        return next;
      });
    };

    const initial = setTimeout(tick, 280);
    const interval = setInterval(tick, 1450);
    return () => {
      clearTimeout(initial);
      clearInterval(interval);
    };
  }, [enabled, demoMode, swarmActive, activeRolesKey]);

  // Expire idle cursors cleanly after inactivity
  useEffect(() => {
    if (!enabled) {
      setCursors({});
      return;
    }
    const timer = setInterval(() => {
      const now = Date.now();
      setCursors((prev) => {
        let changed = false;
        const next: Record<string, ActiveCursorState> = {};
        for (const [id, c] of Object.entries(prev)) {
          if (now - c.lastUpdated <= CURSOR_IDLE_TTL_MS) {
            next[id] = c;
          } else {
            changed = true;
          }
        }
        return changed ? next : prev;
      });
    }, 1500);
    return () => clearInterval(timer);
  }, [enabled]);

  if (!enabled) return null;

  return (
    <div
      aria-hidden="true"
      className="pointer-events-none fixed inset-0 z-40 overflow-hidden"
    >
      {Object.values(cursors).map((cursor) => (
        <div
          key={cursor.agentId}
          className="swarm-cursor absolute top-0 left-0 flex items-start gap-1.5"
          style={{
            transform: `translate3d(${Math.round(cursor.x)}px, ${Math.round(cursor.y)}px, 0)`,
          }}
        >
          <svg
            width="15"
            height="18"
            viewBox="0 0 15 18"
            fill="none"
            className="drop-shadow-[0_2px_6px_rgba(0,0,0,0.85)]"
          >
            <path
              d="M1.5 1.5L13.5 8.5L7.5 10.2L5.2 16.2L1.5 1.5Z"
              fill={cursor.color}
              stroke="#080404"
              strokeWidth="1.4"
              strokeLinejoin="round"
            />
          </svg>
          <div
            className="mt-2.5 flex items-center gap-1.5 rounded-full border border-void/70 px-2 py-0.5 text-[10px] font-semibold tracking-tight text-void shadow-lg"
            style={{ backgroundColor: cursor.color }}
          >
            <span>{cursor.agentId}</span>
            <span className="max-w-[110px] truncate font-normal opacity-80">
              {cursor.label}
            </span>
          </div>
        </div>
      ))}
    </div>
  );
}
