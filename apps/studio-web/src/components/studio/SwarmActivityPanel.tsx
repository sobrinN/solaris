import { useEffect, useMemo, useRef, useState } from "react";
import {
  Paperclip,
  Play,
  ShieldCheck,
  Sparkles,
  Square,
  Terminal,
  X,
} from "lucide-react";
import type {
  AgentRole,
  AgentStatusResponse,
  QaReport,
  SwarmChatTurn,
  SwarmRunMode,
  SwarmRunStatus,
} from "@solaris/engine-client";
import {
  appendDirectorTurn,
  conversationHistory,
  loadDirectorHistory,
  saveDirectorHistory,
  type DirectorTurn,
} from "../../lib/directorHistory";
import { REFINEMENT_CHIPS } from "../../lib/refinementChips";
import { resolveSwarmProgress } from "../../lib/swarmProgress";
import type { SelectedClip } from "../../lib/timelineSelection";

export interface ActivityEntry {
  id: string;
  agentId: string;
  role: AgentRole;
  summary: string;
  timecode: string;
}

export interface StartSwarmOptions {
  prompt: string;
  mode: SwarmRunMode;
  selectedClipIds: string[];
  playheadSec?: number;
  conversationHistory: SwarmChatTurn[];
}

interface SwarmActivityPanelProps {
  projectId: string;
  hasTimelineClips: boolean;
  selectedClips: SelectedClip[];
  playheadSec: number;
  onRemoveSelectedClip?: (clipId: string) => void;
  agentStatus: AgentStatusResponse | null;
  activeModelLabel?: string;
  activeSwarmRun: SwarmRunStatus | null;
  latestQaReport: QaReport | null;
  qaRunning: boolean;
  qaError?: string | null;
  swarmError?: string | null;
  activityLog: ActivityEntry[];
  mediaTabContent?: React.ReactNode;
  mediaCount?: number;
  assetsTabContent?: React.ReactNode;
  assetsCount?: number;
  onAttachFiles?: (files: File[]) => void;
  onOpenLogsDrawer: () => void;
  onStartSwarm: (options: StartSwarmOptions) => Promise<void>;
  onCancelSwarm: () => Promise<void>;
  onRunQa: () => Promise<void>;
}

const ROLE_DOT_COLORS: Record<AgentRole, string> = {
  user: "bg-areia",
  director: "bg-amarelo-ideias",
  razor: "bg-track-v1",
  b_roll: "bg-track-v2",
  grade: "bg-track-fx",
  caps: "bg-track-t1",
  gfx: "bg-track-g1",
  mix: "bg-track-a1",
  mcp_external: "bg-track-a2",
};

function getSessionStorage(): Storage | null {
  try {
    return typeof window !== "undefined" ? window.sessionStorage : null;
  } catch {
    return null;
  }
}

export function SwarmActivityPanel({
  projectId,
  hasTimelineClips,
  selectedClips,
  playheadSec,
  onRemoveSelectedClip,
  agentStatus,
  activeModelLabel,
  activeSwarmRun,
  latestQaReport,
  qaRunning,
  qaError,
  swarmError,
  activityLog,
  mediaTabContent,
  mediaCount,
  assetsTabContent,
  assetsCount,
  onAttachFiles,
  onOpenLogsDrawer,
  onStartSwarm,
  onCancelSwarm,
  onRunQa,
}: SwarmActivityPanelProps) {
  const [prompt, setPrompt] = useState("");
  const [busy, setBusy] = useState(false);
  const [activeTab, setActiveTab] = useState<"prompt" | "activity" | "media" | "assets">("prompt");
  const [omittedClipIds, setOmittedClipIds] = useState<string[]>([]);
  const [history, setHistory] = useState<DirectorTurn[]>(() =>
    loadDirectorHistory(getSessionStorage(), projectId),
  );
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const recordedRunIdRef = useRef<string | null>(null);
  const composerRef = useRef<HTMLTextAreaElement | null>(null);
  const feedEndRef = useRef<HTMLDivElement | null>(null);

  // Load per-project history and reset context overrides on project switch
  useEffect(() => {
    setHistory(loadDirectorHistory(getSessionStorage(), projectId));
    setOmittedClipIds([]);
    recordedRunIdRef.current = null;
  }, [projectId]);

  // Prune omitted clip ids that are no longer in selection
  useEffect(() => {
    const selectedSet = new Set(selectedClips.map((c) => c.clip.id));
    setOmittedClipIds((prev) => prev.filter((id) => selectedSet.has(id)));
  }, [selectedClips]);

  // Append assistant turn to sessionStorage history when a run completes
  useEffect(() => {
    if (!activeSwarmRun || activeSwarmRun.state !== "completed") return;
    if (recordedRunIdRef.current === activeSwarmRun.run_id) return;
    recordedRunIdRef.current = activeSwarmRun.run_id;
    const summaryText =
      activeSwarmRun.refinement_summary?.trim() ||
      activeSwarmRun.summary?.trim() ||
      activeSwarmRun.phase_label;
    if (!summaryText) return;
    setHistory((prev) => {
      const next = appendDirectorTurn(prev, { role: "assistant", text: summaryText });
      saveDirectorHistory(getSessionStorage(), projectId, next);
      return next;
    });
  }, [activeSwarmRun, projectId]);

  useEffect(() => {
    if (activeTab === "prompt" && history.length > 0) {
      feedEndRef.current?.scrollIntoView({ block: "end" });
    }
  }, [history.length, activeTab]);

  const activeContextClips = useMemo(
    () => selectedClips.filter((entry) => !omittedClipIds.includes(entry.clip.id)),
    [selectedClips, omittedClipIds],
  );

  const qaCounts = useMemo(() => {
    if (!latestQaReport) return null;
    let issues = 0;
    let warnings = 0;
    for (const chk of latestQaReport.checks) {
      if (chk.severity === "issue") issues += 1;
      else if (chk.severity === "warning") warnings += 1;
    }
    return { issues, warnings };
  }, [latestQaReport]);

  const isRunning =
    activeSwarmRun !== null &&
    (activeSwarmRun.state === "queued" ||
      activeSwarmRun.state === "planning" ||
      activeSwarmRun.state === "executing_spine" ||
      activeSwarmRun.state === "executing_specialists" ||
      activeSwarmRun.state === "running_qa");

  const swarmProgress = activeSwarmRun ? resolveSwarmProgress(activeSwarmRun) : null;

  const effectiveMode: SwarmRunMode = hasTimelineClips ? "refine" : "generate";

  const handleRun = async () => {
    const trimmed = prompt.trim();
    if (busy || isRunning || !trimmed) return;
    setBusy(true);
    setErrorMsg(null);
    const historyToSend = conversationHistory(history);
    const nextHistory = appendDirectorTurn(history, { role: "user", text: trimmed });
    setHistory(nextHistory);
    saveDirectorHistory(getSessionStorage(), projectId, nextHistory);
    try {
      await onStartSwarm({
        prompt: trimmed,
        mode: effectiveMode,
        selectedClipIds: activeContextClips.map((entry) => entry.clip.id),
        playheadSec: Number(playheadSec.toFixed(2)),
        conversationHistory: historyToSend,
      });
      setPrompt("");
    } catch (err) {
      setErrorMsg(err instanceof Error ? err.message : "Failed to start swarm");
    } finally {
      setBusy(false);
    }
  };

  const tabs = useMemo(() => {
    const base: Array<{ id: "prompt" | "activity" | "media" | "assets"; label: string; count?: number }> = [
      { id: "prompt", label: "Prompt" },
      { id: "activity", label: "Activity", count: activityLog.length > 0 ? activityLog.length : undefined },
    ];
    if (mediaTabContent) {
      base.push({ id: "media", label: "Media", count: mediaCount });
    }
    if (assetsTabContent) {
      base.push({ id: "assets", label: "Assets", count: assetsCount });
    }
    return base;
  }, [activityLog.length, mediaTabContent, mediaCount, assetsTabContent, assetsCount]);

  return (
    <div className="@container solaris-panel flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl">
      {/* Unified Single-Row NLE Panel Header (h-9) */}
      <div className="flex h-9 shrink-0 items-center justify-between gap-1.5 border-b border-areia/[0.08] px-2.5">
        <div className="flex min-w-0 items-center gap-1.5 text-[10px] font-semibold uppercase tracking-[0.14em] text-areia/75">
          <Sparkles className="h-3 w-3 shrink-0 text-amarelo-ideias" />
          <span className="hidden truncate @[345px]:inline">Director</span>
        </div>

        <div className="flex shrink-0 items-center gap-1.5">
          <div
            role="tablist"
            aria-label="Director panel tabs"
            className="flex items-center gap-0.5 rounded-full border border-areia/10 bg-void/75 p-0.5 text-[10px]"
          >
            {tabs.map((tab) => (
              <button
                key={tab.id}
                type="button"
                role="tab"
                aria-selected={activeTab === tab.id}
                onClick={() => setActiveTab(tab.id)}
                className={`flex items-center gap-1 rounded-full px-2 py-0.5 font-medium transition-colors motion-reduce:transition-none ${
                  activeTab === tab.id
                    ? "bg-laranja-criacao/25 text-areia"
                    : "text-areia/55 hover:text-areia"
                }`}
              >
                <span>{tab.label}</span>
                {tab.count !== undefined && tab.count > 0 && (
                  <span className="tabular-nums text-areia/65">{tab.count}</span>
                )}
              </button>
            ))}
          </div>

          <button
            type="button"
            disabled={qaRunning}
            onClick={() => void onRunQa()}
            title="Run quality control and repair issues (frames, -14 LUFS & safe zones)"
            className="flex items-center gap-1 rounded-full border border-areia/12 bg-void/75 px-2 py-0.5 text-[10px] font-medium text-areia/85 transition-colors hover:border-amarelo-ideias/50 hover:bg-accent disabled:opacity-45 motion-reduce:transition-none"
          >
            <ShieldCheck className="h-2.5 w-2.5 text-amarelo-ideias" />
            <span>{qaRunning ? "Repairing..." : "Repair"}</span>
          </button>
        </div>
      </div>

      <div className="flex min-h-0 flex-1 flex-col p-2.5">
      {/* Live Swarm Status Banner */}
      {activeSwarmRun && (
        <div className="mb-2 shrink-0 overflow-hidden rounded-lg border border-laranja-criacao/30 bg-void/80 px-2.5 py-1.5 text-[10px]">
          <div className="flex items-center justify-between gap-2">
            <div className="flex min-w-0 items-center gap-1.5">
              {isRunning && (
                <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-laranja-criacao solar-pulse-dot" />
              )}
              <span
                className="truncate font-medium text-amarelo-ideias"
                title={activeSwarmRun.phase_label}
              >
                {activeSwarmRun.phase_label}
              </span>
            </div>
            <div className="flex shrink-0 items-center gap-1.5 tabular-nums">
              {isRunning && swarmProgress && (
                <span className="font-semibold text-amarelo-ideias">
                  {swarmProgress.percent}%
                </span>
              )}
              <span className="text-areia/60">{activeSwarmRun.ops_committed} ops</span>
            </div>
          </div>
          {isRunning && swarmProgress && (
            <div className="mt-1.5 flex items-center gap-1.5">
              <div
                role="progressbar"
                aria-label="Video build progress"
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={swarmProgress.percent}
                className="h-1.5 flex-1 overflow-hidden rounded-full bg-grafite"
              >
                <div
                  className="h-full w-full origin-left rounded-full bg-gradient-to-r from-laranja-criacao to-amarelo-ideias transition-transform duration-300 ease-out motion-reduce:transition-none"
                  style={{ transform: `scaleX(${swarmProgress.ratio})` }}
                />
              </div>
              {activeSwarmRun.active_roles.slice(0, 4).map((role) => (
                <span
                  key={role}
                  className="inline-flex items-center gap-1 rounded bg-grafite px-1.5 py-0.5 text-[8px] font-semibold uppercase tracking-wider text-areia/80"
                >
                  <span className={`h-1.5 w-1.5 rounded-full ${ROLE_DOT_COLORS[role]}`} />
                  {role}
                </span>
              ))}
            </div>
          )}
          {activeSwarmRun.error && (
            <div className="mt-1.5 rounded border border-vermelho-foco/40 bg-vermelho-foco/15 px-2 py-1 text-areia">
              <div className="break-words">{activeSwarmRun.error}</div>
              <button
                type="button"
                onClick={onOpenLogsDrawer}
                className="mt-1 inline-flex items-center gap-1 rounded bg-vermelho-foco/35 px-1.5 py-0.5 text-[10px] font-medium text-areia hover:bg-vermelho-foco/55"
              >
                <Terminal className="h-2.5 w-2.5" />
                <span>Inspect in Logs</span>
              </button>
            </div>
          )}
        </div>
      )}

      {(errorMsg || swarmError) && (
        <div className="mb-1.5 shrink-0 rounded-lg border border-vermelho-foco/40 bg-vermelho-foco/15 px-2 py-1 text-[10px] text-areia">
          {errorMsg ?? swarmError}
        </div>
      )}
      {qaError && (
        <div className="mb-1.5 shrink-0 rounded-lg border border-laranja-criacao/40 bg-laranja-criacao/15 px-2 py-1 text-[10px] text-areia">
          QA: {qaError}
        </div>
      )}

      {/* TAB 1: PROMPT & REFINEMENT CO-PILOT */}
      {activeTab === "prompt" && (
        <div className="flex min-h-0 flex-1 flex-col gap-2">
          {/* Compact QA Health Strip (click to inspect full breakdown in Activity) */}
          {latestQaReport && qaCounts && (
            <button
              type="button"
              onClick={() => setActiveTab("activity")}
              className="flex shrink-0 items-center justify-between gap-2 rounded-lg border border-areia/10 bg-void/65 px-2.5 py-1.5 text-left text-[10px] transition-colors hover:border-amarelo-ideias/40 hover:bg-accent motion-reduce:transition-none"
            >
              <div className="flex items-center gap-1.5">
                <span className="font-semibold text-areia/85">QA</span>
                <span
                  className={`tabular-nums font-semibold ${
                    latestQaReport.passed ? "text-amarelo-ideias" : "text-laranja-criacao"
                  }`}
                >
                  {latestQaReport.score}/100
                </span>
                {qaCounts.issues > 0 && (
                  <span className="tabular-nums rounded bg-vermelho-foco px-1.5 py-0.5 text-[9px] font-semibold uppercase text-areia">
                    {qaCounts.issues} {qaCounts.issues === 1 ? "issue" : "issues"}
                  </span>
                )}
                {qaCounts.warnings > 0 && (
                  <span className="tabular-nums rounded bg-laranja-criacao/25 px-1.5 py-0.5 text-[9px] font-medium text-areia/90">
                    {qaCounts.warnings} warn
                  </span>
                )}
              </div>
              <span className="tabular-nums text-areia/50">
                {latestQaReport.frames_inspected}f · {latestQaReport.auto_fixes_applied} fixed
              </span>
            </button>
          )}

          {/* Living Director Feed (fills middle height cleanly instead of empty void) */}
          <div className="solaris-scroll min-h-0 flex-1 space-y-1.5 overflow-y-auto pr-0.5">
            {history.length > 0 ? (
              <>
                {history.map((turn, idx) => (
                  <div
                    key={`${turn.role}-${idx}`}
                    className={`rounded-lg border px-2.5 py-1.5 text-[11px] leading-snug ${
                      turn.role === "user"
                        ? "border-areia/10 border-l-2 border-l-laranja-criacao bg-void/65 text-areia/95"
                        : "border-areia/[0.07] border-l-2 border-l-amarelo-ideias/70 bg-void/40 text-areia/75"
                    }`}
                  >
                    <div className="mb-0.5 text-[9px] font-semibold uppercase tracking-[0.12em] text-areia/45">
                      {turn.role === "user" ? "You" : "Director"}
                    </div>
                    <p className="break-words">{turn.text}</p>
                  </div>
                ))}
                <div ref={feedEndRef} />
              </>
            ) : activityLog.length > 0 ? (
              <div className="space-y-1">
                <div className="px-0.5 text-[9px] font-semibold uppercase tracking-[0.14em] text-areia/40">
                  Recent Swarm Ops
                </div>
                {activityLog.slice(0, 6).map((entry) => (
                  <div
                    key={entry.id}
                    className="flex items-center gap-1.5 rounded-md border border-areia/[0.06] bg-void/45 px-2 py-1 text-[10px]"
                  >
                    <span
                      className={`h-1.5 w-1.5 shrink-0 rounded-full ${ROLE_DOT_COLORS[entry.role] ?? "bg-areia/40"}`}
                    />
                    <span className="shrink-0 rounded bg-grafite px-1.5 py-0.5 text-[9px] font-semibold text-amarelo-ideias">
                      {entry.agentId}
                    </span>
                    <span className="min-w-0 flex-1 truncate text-areia/80">
                      {entry.summary}
                    </span>
                    <span className="tabular-nums shrink-0 text-[9px] text-areia/35">
                      {entry.timecode}
                    </span>
                  </div>
                ))}
              </div>
            ) : (
              <div className="flex h-full min-h-[72px] flex-col items-center justify-center rounded-lg border border-dashed border-areia/[0.08] bg-void/30 px-4 text-center">
                <Sparkles className="mb-1.5 h-4 w-4 text-laranja-criacao/80" />
                <p className="text-[11px] font-medium text-areia/75">
                  {effectiveMode === "refine"
                    ? "Select clips or type a direction to refine this cut"
                    : "Describe the video to direct the Solaris swarm"}
                </p>
                <p className="mt-0.5 text-[10px] text-areia/40">
                  {agentStatus?.openrouter_configured
                    ? `Model: ${activeModelLabel ?? "OpenRouter"}`
                    : "Local fallback mode"}
                </p>
              </div>
            )}
          </div>

          {/* Bottom-Pinned Co-Pilot Composer */}
          <div className="shrink-0 space-y-1.5 border-t border-areia/[0.08] pt-2">
            {/* Context Pills (selected clips) */}
            {activeContextClips.length > 0 && (
              <div className="flex flex-wrap items-center gap-1">
                {activeContextClips.map(({ clip }) => (
                  <span
                    key={clip.id}
                    className="inline-flex max-w-[148px] items-center gap-1 rounded-full border border-amarelo-ideias/35 bg-amarelo-ideias/10 px-2 py-0.5 text-[10px] text-areia"
                  >
                    <span className="truncate" title={clip.label}>
                      {clip.label}
                    </span>
                    <button
                      type="button"
                      aria-label={`Remove ${clip.label} from prompt context`}
                      onClick={() => {
                        setOmittedClipIds((prev) => [...prev, clip.id]);
                        onRemoveSelectedClip?.(clip.id);
                      }}
                      className="text-areia/65 hover:text-areia"
                    >
                      <X className="h-2.5 w-2.5" />
                    </button>
                  </span>
                ))}
              </div>
            )}

            {/* Multi-line Composer */}
            <div className="rounded-lg border border-areia/12 bg-void/85 p-2 focus-within:border-laranja-criacao">
              <textarea
                ref={composerRef}
                rows={3}
                disabled={isRunning}
                value={prompt}
                onChange={(e) => setPrompt(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && !isRunning && !busy) {
                    e.preventDefault();
                    void handleRun();
                  }
                }}
                placeholder={
                  effectiveMode === "refine"
                    ? "Describe what to adjust (⌘/Ctrl+Enter)..."
                    : agentStatus?.openrouter_configured
                      ? `Direct swarm (${activeModelLabel ?? "OpenRouter"})...`
                      : "Direct swarm (Local mode)..."
                }
                className="w-full resize-none bg-transparent text-[11px] leading-snug text-areia placeholder:text-areia/35 outline-none disabled:opacity-50"
              />
              <div className="mt-1.5 flex items-center justify-between gap-2">
                <div className="flex items-center gap-2">
                  {onAttachFiles && (
                    <label className="inline-flex h-6 cursor-pointer items-center gap-1 rounded-md border border-areia/12 bg-carbono/80 px-2 text-[10px] text-areia/80 transition-colors hover:border-laranja-criacao/45 hover:text-areia motion-reduce:transition-none">
                      <Paperclip className="h-3 w-3 text-laranja-criacao" />
                      <span>Attach</span>
                      <input
                        type="file"
                        multiple
                        accept="video/*,image/*,audio/*,.pdf,.md,.txt"
                        className="hidden"
                        onChange={(e) => {
                          if (e.target.files && e.target.files.length > 0) {
                            onAttachFiles(Array.from(e.target.files));
                          }
                          e.currentTarget.value = "";
                        }}
                      />
                    </label>
                  )}
                  <span className="text-[10px] text-areia/40">⌘/Ctrl+Enter</span>
                </div>
                {isRunning ? (
                  <button
                    type="button"
                    onClick={() => void onCancelSwarm()}
                    className="flex h-6 items-center gap-1 rounded-md border border-vermelho-foco/60 bg-vermelho-foco/25 px-2.5 text-[10px] font-semibold text-areia hover:bg-vermelho-foco/40"
                  >
                    <Square className="h-2.5 w-2.5 fill-current" />
                    <span>Cancel</span>
                  </button>
                ) : (
                  <button
                    type="button"
                    disabled={busy || !prompt.trim()}
                    onClick={() => void handleRun()}
                    className="flex h-6 items-center gap-1 rounded-md bg-primary px-2.5 text-[10px] font-semibold text-areia transition-opacity disabled:opacity-40 motion-reduce:transition-none"
                  >
                    <Play className="h-2.5 w-2.5 fill-current" />
                    <span>{effectiveMode === "refine" ? "Refine" : "Direct"}</span>
                  </button>
                )}
              </div>
            </div>

            {/* Quick Refinement Chips */}
            {hasTimelineClips && (
              <div className="flex flex-wrap gap-1">
                {REFINEMENT_CHIPS.map((chip) => (
                  <button
                    key={chip.id}
                    type="button"
                    disabled={isRunning}
                    onClick={() => {
                      setPrompt(chip.prompt);
                      composerRef.current?.focus();
                    }}
                    className="rounded-md border border-areia/10 bg-void/65 px-2 py-0.5 text-[10px] text-areia/70 transition-colors hover:border-laranja-criacao/50 hover:bg-accent hover:text-areia disabled:opacity-40 motion-reduce:transition-none"
                  >
                    {chip.label}
                  </button>
                ))}
              </div>
            )}
          </div>
        </div>
      )}

      {/* TAB 2: ACTIVITY & QA */}
      {activeTab === "activity" && (
        <div className="solaris-scroll flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto pr-0.5">
          {latestQaReport && (
            <div className="rounded-lg border border-areia/12 bg-void/75 px-2.5 py-2 text-[10px]">
              <div className="flex items-center justify-between gap-2">
                <span className="font-semibold text-areia">
                  QA Score{" "}
                  <span className="tabular-nums text-amarelo-ideias">
                    {latestQaReport.score}/100
                  </span>
                </span>
                <span className="tabular-nums text-areia/60">
                  {latestQaReport.frames_inspected}f · {latestQaReport.auto_fixes_applied} fixed
                </span>
              </div>
              {latestQaReport.multimodal_summary && (
                <p className="mt-1 break-words leading-snug text-[10px] text-areia/75">
                  {latestQaReport.multimodal_summary}
                </p>
              )}
              {latestQaReport.checks.length > 0 && (
                <div className="solaris-scroll mt-1.5 max-h-44 space-y-1 overflow-y-auto pr-0.5">
                  {[...latestQaReport.checks]
                    .sort((a, b) => {
                      const rank = { issue: 0, warning: 1, fixed: 2, pass: 3 };
                      return rank[a.severity] - rank[b.severity];
                    })
                    .map((chk) => (
                      <div
                        key={chk.id}
                        className="flex items-start gap-1.5 text-[10px] text-areia/85"
                      >
                        <span
                          className={`mt-0.5 shrink-0 rounded px-1 py-0.5 text-[9px] font-semibold uppercase leading-none ${
                            chk.severity === "issue"
                              ? "bg-vermelho-foco text-areia"
                              : chk.severity === "warning"
                                ? "bg-laranja-criacao/30 text-areia"
                                : chk.severity === "fixed"
                                  ? "bg-amarelo-ideias/25 text-amarelo-ideias"
                                  : "bg-grafite text-areia/75"
                          }`}
                        >
                          {chk.severity}
                        </span>
                        <span className="min-w-0 flex-1 break-words leading-snug">
                          {chk.message}
                        </span>
                      </div>
                    ))}
                </div>
              )}
            </div>
          )}

          <div className="space-y-1 text-[11px]">
            {activityLog.length === 0 ? (
              <div className="rounded-lg border border-dashed border-areia/[0.08] bg-void/30 px-3 py-4 text-center text-[10px] text-areia/40">
                Waiting for agent or user edits...
              </div>
            ) : (
              activityLog.map((entry) => (
                <div
                  key={entry.id}
                  className="flex items-center gap-2 rounded-lg border border-areia/[0.06] bg-void/55 px-2 py-1.5"
                >
                  <span
                    className={`h-1.5 w-1.5 shrink-0 rounded-full ${ROLE_DOT_COLORS[entry.role] ?? "bg-areia/40"}`}
                  />
                  <span className="tabular-nums shrink-0 text-[10px] text-areia/40">
                    {entry.timecode}
                  </span>
                  <span className="shrink-0 rounded bg-grafite px-1.5 py-0.5 text-[10px] font-semibold text-amarelo-ideias">
                    {entry.agentId}
                  </span>
                  <span className="min-w-0 flex-1 truncate text-areia/85">{entry.summary}</span>
                </div>
              ))
            )}
          </div>
        </div>
      )}

      {/* TAB 3: MEDIA BIN */}
      {activeTab === "media" && mediaTabContent && (
        <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
          {mediaTabContent}
        </div>
      )}

      {/* TAB 4: ASSETS (GFX BIN) */}
      {activeTab === "assets" && assetsTabContent && (
        <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
          {assetsTabContent}
        </div>
      )}
      </div>
    </div>
  );
}
