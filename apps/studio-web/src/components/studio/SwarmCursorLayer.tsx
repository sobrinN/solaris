import { useEffect, useRef, useState } from "react";
import type { AgentRole, EngineEventEnvelope } from "@solaris/engine-client";

export interface ActiveCursorState {
  agentId: string;
  role: AgentRole;
  anchor: string;
  label: string;
  color: string;
  x: number;
  y: number;
  lastUpdated: number;
  lastRealEventAt: number;
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

const PARALLEL_GFX_PALETTE = [
  "#FB7185",
  "#F05A00",
  "#F7B733",
  "#38BDF8",
  "#A78BFA",
  "#34D399",
  "#F472B6",
];

function colorForAgent(agentId: string, role: AgentRole): string {
  if (role === "gfx") {
    const match = /-(\d+)$/.exec(agentId);
    if (match) {
      const idx = Math.max(0, Number.parseInt(match[1], 10) - 1);
      return PARALLEL_GFX_PALETTE[idx % PARALLEL_GFX_PALETTE.length];
    }
  }
  return ROLE_COLORS[role] ?? "#F05A00";
}

const CHOREOGRAPHY_ACTORS: Array<{
  agentId: string;
  role: AgentRole;
  anchors: string[];
  xBias: number;
  label: string;
}> = [
  { agentId: "razor-01", role: "razor", anchors: ["timeline.track.v1"], xBias: 0.22, label: "trimming breath" },
  { agentId: "caps-01", role: "caps", anchors: ["timeline.track.t1", "captions.inspector"], xBias: 0.36, label: "syncing captions" },
  { agentId: "grade-01", role: "grade", anchors: ["color.inspector", "scopes.panel"], xBias: 0.42, label: "balancing grade" },
  { agentId: "gfx-01", role: "gfx", anchors: ["timeline.track.g1", "effects.library"], xBias: 0.48, label: "crafting motion" },
  { agentId: "broll-01", role: "b_roll", anchors: ["timeline.track.v2", "media.pool"], xBias: 0.62, label: "syncing insert" },
  { agentId: "mix-01", role: "mix", anchors: ["timeline.track.a2", "audio.mixer"], xBias: 0.72, label: "ducking -18 dB" },
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
  totalDurationSec = 24,
  microJitter = false,
): { x: number; y: number } | null {
  const [baseAnchor, secStr] = anchor.split("@");
  const parsedSec = secStr !== undefined ? Number.parseFloat(secStr) : Number.NaN;
  const vw = typeof window !== "undefined" ? window.innerWidth : 1440;
  const vh = typeof window !== "undefined" ? window.innerHeight : 900;

  // Exact timecode positioning on a timeline track lane (`timeline.track.g1@6.50`)
  if (baseAnchor.startsWith("timeline.track.") && Number.isFinite(parsedSec) && parsedSec >= 0) {
    const trackEl = document.querySelector<HTMLElement>(
      `[data-cursor-anchor="${escapeAttr(baseAnchor)}"]`,
    );
    const laneEl = trackEl?.querySelector<HTMLElement>("[data-timeline-lane]") ?? trackEl;
    if (laneEl) {
      const rect = laneEl.getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0) {
        const timelineSpan =
          totalDurationSec > 0 ? Math.max(12, Math.ceil(totalDurationSec + 2)) : 20;
        const ratio = Math.max(0.02, Math.min(0.95, parsedSec / timelineSpan));
        const minX = Math.max(12, rect.left + 6);
        const maxX = Math.max(minX, Math.min(rect.right - 140, vw - 200));
        const minY = Math.max(56, rect.top + 4);
        const maxY = Math.max(minY, Math.min(rect.bottom - 24, vh - 54));
        const dx = microJitter ? (Math.random() - 0.5) * 28 : 0;
        const dy = microJitter ? (Math.random() - 0.5) * 10 : 0;
        return {
          x: Math.max(minX, Math.min(maxX, rect.left + rect.width * ratio + dx)),
          y: Math.max(minY, Math.min(maxY, rect.top + rect.height * 0.45 + dy)),
        };
      }
    }
  }

  const prefix = baseAnchor.split(".").slice(0, 2).join(".");
  const rawEl =
    document.querySelector<HTMLElement>(`[data-cursor-anchor="${escapeAttr(baseAnchor)}"]`) ??
    document.querySelector<HTMLElement>(`[data-cursor-anchor="${escapeAttr(prefix)}"]`) ??
    document.querySelector<HTMLElement>(`[data-cursor-anchor="timeline"]`);
  if (!rawEl) return null;
  // Target the timeline lane directly when anchoring to a track row so cursors never cover mute/solo/lock headers
  const el = baseAnchor.startsWith("timeline.track.")
    ? (rawEl.querySelector<HTMLElement>("[data-timeline-lane]") ?? rawEl)
    : rawEl;
  const rect = el.getBoundingClientRect();
  if (rect.width === 0 || rect.height === 0) return null;

  const spreadX = rect.width * (0.1 + xBias * 0.6);
  const jitterX = microJitter ? (Math.random() - 0.5) * Math.min(rect.width * 0.16, 56) : 0;
  const jitterY = microJitter ? (Math.random() - 0.5) * Math.min(rect.height * 0.35, 22) : 0;
  const minX = Math.max(12, rect.left + 8);
  const maxX = Math.max(minX, Math.min(rect.right - 150, vw - 210));
  const minY = Math.max(56, rect.top + 8);
  const maxY = Math.max(minY, Math.min(rect.bottom - 34, vh - 54));
  return {
    x: Math.max(minX, Math.min(maxX, rect.left + spreadX + jitterX)),
    y: Math.max(minY, Math.min(maxY, rect.top + rect.height * 0.5 + jitterY)),
  };
}

interface SwarmCursorLayerProps {
  enabled: boolean;
  swarmActive?: boolean;
  activeRoles?: AgentRole[];
  events?: EngineEventEnvelope[];
  totalDurationSec?: number;
}

const CURSOR_IDLE_TTL_MS = 5200;

export function SwarmCursorLayer({
  enabled,
  swarmActive = false,
  activeRoles = [],
  events = [],
  totalDurationSec = 24,
}: SwarmCursorLayerProps) {
  const [cursors, setCursors] = useState<Record<string, ActiveCursorState>>({});
  const cursorsRef = useRef<Record<string, ActiveCursorState>>(cursors);
  cursorsRef.current = cursors;
  const lastProcessedEventRef = useRef<EngineEventEnvelope | null>(null);

  // Drain EVERY newly arrived op_committed & cursor_pulse event in the batch so all
  // parallel micro-agents update simultaneously instead of dropping sibling events.
  useEffect(() => {
    if (events.length === 0) {
      lastProcessedEventRef.current = null;
      return;
    }
    if (!enabled) {
      return;
    }

    const lastSeen = lastProcessedEventRef.current;
    let newEvents: EngineEventEnvelope[];
    if (lastSeen === null) {
      newEvents = events.slice(-24);
    } else {
      let foundIdx = -1;
      for (let i = events.length - 1; i >= 0; i--) {
        if (events[i] === lastSeen) {
          foundIdx = i;
          break;
        }
      }
      newEvents = foundIdx >= 0 ? events.slice(foundIdx + 1) : events.slice(-24);
    }
    lastProcessedEventRef.current = events[events.length - 1] ?? null;
    if (newEvents.length === 0) return;

    const now = Date.now();
    const batchUpdates: Array<{
      agentId: string;
      role: AgentRole;
      anchor: string;
      label: string;
      isCommit: boolean;
    }> = [];

    for (let i = 0; i < newEvents.length; i++) {
      const ev = newEvents[i];
      const ts = Date.parse(ev.timestamp);
      if (Number.isFinite(ts) && now - ts > CURSOR_IDLE_TTL_MS) continue;
      const p = ev.payload;
      if (p.event === "op_committed" && p.role !== "user") {
        // Parallel scene micro-agents (gfx-01..07) commit through the shared "gfx-swarm" lock ID
        // and emit their own per-agent cursor_pulse with exact @sec coordinates; skip the lock alias.
        if (p.agent_id === "gfx-swarm") continue;
        batchUpdates.push({
          agentId: p.agent_id,
          role: p.role,
          anchor: p.ui_anchor,
          label: p.summary.slice(0, 26),
          isCommit: true,
        });
      } else if (p.event === "cursor_pulse" && p.role !== "user") {
        batchUpdates.push({
          agentId: p.agent_id,
          role: p.role,
          anchor: p.ui_anchor,
          label: p.label.slice(0, 26),
          isCommit: p.label.startsWith("placed "),
        });
      }
    }

    if (batchUpdates.length === 0) return;

    const reducedMotion =
      typeof window !== "undefined" &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const resolvedUpdates: ActiveCursorState[] = [];
    for (const upd of batchUpdates) {
      const actorBias =
        CHOREOGRAPHY_ACTORS.find((a) => a.role === upd.role)?.xBias ?? 0.5;
      // Add subtle live work micro-motion on streaming pulses while snapping accurately on commits
      const coords = resolveAnchorCoordinates(
        upd.anchor,
        actorBias,
        totalDurationSec,
        !upd.isCommit && !reducedMotion,
      );
      if (!coords) continue;
      resolvedUpdates.push({
        agentId: upd.agentId,
        role: upd.role,
        anchor: upd.anchor,
        label: upd.label,
        color: colorForAgent(upd.agentId, upd.role),
        x: coords.x,
        y: coords.y,
        lastUpdated: now,
        lastRealEventAt: now,
      });
    }

    if (resolvedUpdates.length === 0) return;

    setCursors((prev) => {
      const next = { ...prev };
      for (const item of resolvedUpdates) {
        next[item.agentId] = item;
      }
      return next;
    });
  }, [enabled, events, totalDurationSec]);

  const activeRolesKey = activeRoles.join(",");

  // High-responsiveness scanning micro-motions during real Swarm LLM streaming gaps
  useEffect(() => {
    if (!enabled || !swarmActive) return;
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (reduced) return;

    const roleList = activeRolesKey ? (activeRolesKey.split(",") as AgentRole[]) : [];

    const tick = () => {
      const now = Date.now();
      const matchedActors =
        roleList.length > 0
          ? CHOREOGRAPHY_ACTORS.filter((a) => roleList.includes(a.role))
          : [];
      const targetActors =
        matchedActors.length > 0
          ? matchedActors
          : [
              {
                agentId: "director-01",
                role: "director" as AgentRole,
                anchors: ["media.pool", "timeline", "effects.library"],
                xBias: 0.45,
                label: "orchestrating",
              },
            ];

      // Resolve DOM coordinates outside setCursors so state updaters remain pure in React StrictMode
      const snapshot = cursorsRef.current;
      const driftPatches: Record<string, { x: number; y: number }> = {};
      for (const [id, cur] of Object.entries(snapshot)) {
        if (now - cur.lastUpdated >= 520 && now - cur.lastRealEventAt < CURSOR_IDLE_TTL_MS) {
          const coords = resolveAnchorCoordinates(cur.anchor, 0.5, totalDurationSec, true);
          if (coords) {
            driftPatches[id] = coords;
          }
        }
      }

      const resolvedActors: Array<{
        agentId: string;
        role: AgentRole;
        anchor: string;
        label: string;
        color: string;
        x: number;
        y: number;
      }> = [];
      for (const actor of targetActors) {
        const anchor = actor.anchors[Math.floor(Math.random() * actor.anchors.length)];
        const coords = resolveAnchorCoordinates(anchor, actor.xBias, totalDurationSec, true);
        if (!coords) continue;
        resolvedActors.push({
          agentId: actor.agentId,
          role: actor.role,
          anchor,
          label: actor.label,
          color: colorForAgent(actor.agentId, actor.role),
          x: coords.x,
          y: coords.y,
        });
      }

      setCursors((prev) => {
        const next: Record<string, ActiveCursorState> = { ...prev };
        // Gently drift live parallel micro-agents (e.g. gfx-02..07) without extending lastRealEventAt
        for (const [id, coords] of Object.entries(driftPatches)) {
          const cur = prev[id];
          if (
            cur &&
            now - cur.lastUpdated >= 520 &&
            now - cur.lastRealEventAt < CURSOR_IDLE_TTL_MS
          ) {
            next[id] = {
              ...cur,
              x: coords.x,
              y: coords.y,
              lastUpdated: now,
            };
          }
        }
        for (const actor of resolvedActors) {
          // If parallel scene micro-agents (gfx-02+) are already live, don't duplicate a synthetic gfx-01
          if (
            actor.role === "gfx" &&
            Object.keys(next).some((k) => k.startsWith("gfx-") && k !== actor.agentId)
          ) {
            continue;
          }
          const existing = next[actor.agentId];
          if (existing && now - existing.lastUpdated < 520) continue;
          next[actor.agentId] = {
            agentId: actor.agentId,
            role: actor.role,
            anchor: actor.anchor,
            label: existing?.label ?? actor.label,
            color: actor.color,
            x: actor.x,
            y: actor.y,
            lastUpdated: now,
            lastRealEventAt: now,
          };
        }
        return next;
      });
    };

    const initial = setTimeout(tick, 80);
    const interval = setInterval(tick, 450);
    return () => {
      clearTimeout(initial);
      clearInterval(interval);
    };
  }, [enabled, swarmActive, activeRolesKey, totalDurationSec]);

  // Expire idle cursors cleanly after inactivity (tracked via lastRealEventAt)
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
          if (now - c.lastRealEventAt <= CURSOR_IDLE_TTL_MS) {
            next[id] = c;
          } else {
            changed = true;
          }
        }
        return changed ? next : prev;
      });
    }, 1200);
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
