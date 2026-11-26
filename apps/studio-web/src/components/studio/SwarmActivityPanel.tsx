import { useEffect, useMemo, useRef, useState } from "react";
import {
  Check,
  Copy,
  Maximize2,
  Play,
  ShieldCheck,
  Sparkles,
  Square,
  Terminal,
  Trash2,
  X,
} from "lucide-react";
import type {
  AgentRole,
  AgentStatusResponse,
  LogEntry,
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
import type { SelectedClip } from "../../lib/timelineSelection";
import { DiagnosticLogList } from "./DiagnosticLogList";
import { formatLogsForCopy } from "./LogsDrawer";

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
  logs: LogEntry[];
  onClearLogs: () => Promise<void>;
  onOpenLogsDrawer: () => void;
  onStartSwarm: (options: StartSwarmOptions) => Promise<void>;
  onCancelSwarm: () => Promise<void>;
  onRunQa: () => Promise<void>;
}

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
  logs,
  onClearLogs,
  onOpenLogsDrawer,
  onStartSwarm,
  onCancelSwarm,
  onRunQa,
}: SwarmActivityPanelProps) {
  const [prompt, setPrompt] = useState("");
  const [busy, setBusy] = useState(false);
  const [copiedLogs, setCopiedLogs] = useState(false);
  const [clearingLogs, setClearingLogs] = useState(false);
  const [activeTab, setActiveTab] = useState<"prompt" | "activity" | "logs">("prompt");
  const [logFilter, setLogFilter] = useState<"all" | "error" | "warn">("all");
  const [cutMode, setCutMode] = useState<"refine" | "generate">(() =>
    hasTimelineClips ? "refine" : "generate",
  );
  const [omittedClipIds, setOmittedClipIds] = useState<string[]>([]);
  const [history, setHistory] = useState<DirectorTurn[]>(() =>
    loadDirectorHistory(getSessionStorage(), projectId),
  );
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const copyLogsTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const recordedRunIdRef = useRef<string | null>(null);
  const composerRef = useRef<HTMLTextAreaElement | null>(null);

  useEffect(() => {
    return () => {
      if (copyLogsTimerRef.current) clearTimeout(copyLogsTimerRef.current);
    };
  }, []);

  // Load per-project history and reset context overrides on project switch
  useEffect(() => {
    setHistory(loadDirectorHistory(getSessionStorage(), projectId));
    setOmittedClipIds([]);
    setCutMode(hasTimelineClips ? "refine" : "generate");
    recordedRunIdRef.current = null;
  }, [projectId]);

  // Switch to refine automatically once clips exist if user hasn't forced generate on an empty cut
  useEffect(() => {
    if (!hasTimelineClips && cutMode === "refine") {
      setCutMode("generate");
    }
  }, [hasTimelineClips, cutMode]);

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

  const activeContextClips = useMemo(
    () => selectedClips.filter((entry) => !omittedClipIds.includes(entry.clip.id)),
    [selectedClips, omittedClipIds],
  );

  const errorLogCount = useMemo(
    () => logs.filter((l) => l.level === "error").length,
    [logs],
  );

  const visibleLogs = useMemo(() => {
    return [...logs]
      .reverse()
      .filter((entry) => {
        if (logFilter === "error") return entry.level === "error";
        if (logFilter === "warn") return entry.level === "warn" || entry.level === "error";
        return true;
      })
      .slice(0, 120);
  }, [logs, logFilter]);

  const isRunning =
    activeSwarmRun !== null &&
    (activeSwarmRun.state === "queued" ||
      activeSwarmRun.state === "planning" ||
      activeSwarmRun.state === "executing_spine" ||
      activeSwarmRun.state === "executing_specialists" ||
      activeSwarmRun.state === "running_qa");

  const effectiveMode: SwarmRunMode = hasTimelineClips ? cutMode : "generate";

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

  const handleCopyLogs = () => {
    const text = formatLogsForCopy([...visibleLogs].reverse());
    if (!text) return;
    if (!navigator.clipboard?.writeText) {
      setErrorMsg("Clipboard unavailable in this browser context.");
      return;
    }
    void navigator.clipboard
      .writeText(text)
      .then(() => {
        setCopiedLogs(true);
        if (copyLogsTimerRef.current) clearTimeout(copyLogsTimerRef.current);
        copyLogsTimerRef.current = setTimeout(() => setCopiedLogs(false), 1600);
      })
      .catch(() => setErrorMsg("Failed to copy logs to clipboard."));
  };

  const handleClearLogs = async () => {
    if (clearingLogs) return;
    setClearingLogs(true);
    setErrorMsg(null);
    try {
      await onClearLogs();
    } catch (err) {
      setErrorMsg(err instanceof Error ? err.message : "Failed to clear logs");
    } finally {
      setClearingLogs(false);
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col rounded-2xl border border-areia/10 bg-carbono/70 p-3">
      {/* Header */}
      <div className="mb-2 flex flex-wrap items-center justify-between gap-1.5 text-[10px] font-semibold uppercase tracking-[0.14em] text-areia/60">
        <div className="flex items-center gap-1.5">
          <Sparkles className="h-3 w-3 text-amarelo-ideias" />
          <span>Director &amp; Swarm</span>
        </div>
        <div className="flex flex-wrap items-center gap-1">
          <button
            type="button"
            disabled={qaRunning}
            onClick={() => void onRunQa()}
            title="Run quality control and auto-fix issues (frames, -14 LUFS & safe zones)"
            className="flex items-center gap-1 rounded-full border border-areia/12 bg-void/65 px-2 py-0.5 text-[10px] font-medium normal-case tracking-normal text-areia/80 hover:border-amarelo-ideias/50 hover:bg-accent disabled:opacity-45"
          >
            <ShieldCheck className="h-2.5 w-2.5 text-amarelo-ideias" />
            <span>{qaRunning ? "Enhancing..." : "Enhance"}</span>
          </button>
        </div>
      </div>

      {/* Prompt | Activity | Logs Tabs */}
      <div className="mb-2 flex items-center justify-between gap-1.5 border-y border-areia/10 py-1.5">
        <div
          role="tablist"
          aria-label="Director panel tabs"
          className="flex items-center gap-0.5 rounded-full border border-areia/10 bg-void/65 p-0.5 text-[10px]"
        >
          {(
            [
              ["prompt", "Prompt"],
              ["activity", "Activity"],
              ["logs", "Logs"],
            ] as const
          ).map(([id, label]) => (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={activeTab === id}
              onClick={() => setActiveTab(id)}
              className={`flex items-center gap-1 rounded-full px-2 py-0.5 font-medium transition-colors motion-reduce:transition-none ${
                activeTab === id
                  ? "bg-laranja-criacao/25 text-areia"
                  : "text-areia/55 hover:text-areia"
              }`}
            >
              <span>{label}</span>
              {id === "activity" && activityLog.length > 0 && (
                <span className="tabular-nums text-areia/65">{activityLog.length}</span>
              )}
              {id === "logs" && errorLogCount > 0 && (
                <span className="tabular-nums rounded-full bg-vermelho-foco px-1.5 text-[10px] font-semibold text-areia">
                  {errorLogCount}
                </span>
              )}
            </button>
          ))}
        </div>

        <div className="flex items-center gap-1">
          {activeTab === "logs" && (
            <>
              <button
                type="button"
                disabled={visibleLogs.length === 0}
                onClick={handleCopyLogs}
                title="Copy visible logs"
                aria-label="Copy visible logs"
                className="grid h-6 w-6 place-items-center rounded-md border border-areia/12 bg-void/65 text-areia/75 hover:border-amarelo-ideias/50 hover:text-areia disabled:opacity-40"
              >
                {copiedLogs ? (
                  <Check className="h-2.5 w-2.5 text-amarelo-ideias" />
                ) : (
                  <Copy className="h-2.5 w-2.5" />
                )}
              </button>
              <button
                type="button"
                disabled={logs.length === 0 || clearingLogs}
                onClick={() => void handleClearLogs()}
                title="Clear logs"
                aria-label="Clear logs"
                className="grid h-6 w-6 place-items-center rounded-md border border-areia/12 bg-void/65 text-areia/75 hover:border-vermelho-foco/55 hover:text-areia disabled:opacity-40"
              >
                <Trash2 className="h-2.5 w-2.5" />
              </button>
            </>
          )}
          <button
            type="button"
            onClick={onOpenLogsDrawer}
            title="Expand full-width Diagnostic Logs drawer"
            aria-label="Expand diagnostic logs"
            className="grid h-6 w-6 place-items-center rounded-md border border-areia/12 bg-void/65 text-areia/75 hover:border-laranja-criacao/50 hover:text-areia"
          >
            <Maximize2 className="h-2.5 w-2.5" />
          </button>
        </div>
      </div>

      {/* Live Swarm Status Banner */}
      {activeSwarmRun && (
        <div className="mb-2 rounded-lg border border-laranja-criacao/30 bg-void/70 px-2 py-1.5 text-[10px]">
          <div className="flex items-center justify-between gap-2">
            <span className="truncate font-medium text-amarelo-ideias">
              {activeSwarmRun.phase_label}
            </span>
            <span className="tabular-nums shrink-0 text-areia/65">
              {activeSwarmRun.ops_committed} ops
            </span>
          </div>
          {activeSwarmRun.error && (
            <div className="mt-1 rounded border border-vermelho-foco/40 bg-vermelho-foco/15 px-1.5 py-1 text-areia">
              <div className="break-words">{activeSwarmRun.error}</div>
              <button
                type="button"
                onClick={() => {
                  setActiveTab("logs");
                  setLogFilter("error");
                }}
                className="mt-1 inline-flex items-center gap-1 rounded bg-vermelho-foco/35 px-1.5 py-0.5 text-[10px] font-medium text-areia hover:bg-vermelho-foco/55"
              >
                <Terminal className="h-2.5 w-2.5" />
                <span>Inspect full error in Logs</span>
              </button>
            </div>
          )}
        </div>
      )}

      {(errorMsg || swarmError) && (
        <div className="mb-1.5 rounded-lg border border-vermelho-foco/40 bg-vermelho-foco/15 px-2 py-1 text-[10px] text-areia">
          {errorMsg ?? swarmError}
        </div>
      )}
      {qaError && (
        <div className="mb-1.5 rounded-lg border border-laranja-criacao/40 bg-laranja-criacao/15 px-2 py-1 text-[10px] text-areia">
          QA: {qaError}
        </div>
      )}

      {/* TAB 1: PROMPT & REFINEMENT COMPOSER */}
      {activeTab === "prompt" && (
        <div className="flex min-h-0 flex-1 flex-col gap-2">
          {/* Cut Mode Toggle */}
          <div className="flex items-center justify-between gap-1.5">
            <div
              role="group"
              aria-label="Cut mode"
              className="flex items-center gap-0.5 rounded-full border border-areia/10 bg-void/70 p-0.5 text-[10px]"
            >
              <button
                type="button"
                disabled={!hasTimelineClips}
                onClick={() => setCutMode("refine")}
                title={
                  hasTimelineClips
                    ? "Targeted edits to existing clips without regenerating the whole cut"
                    : "Generate a cut first to enable Refine mode"
                }
                className={`rounded-full px-2 py-0.5 font-medium transition-colors disabled:opacity-35 motion-reduce:transition-none ${
                  effectiveMode === "refine"
                    ? "bg-laranja-criacao/25 text-areia"
                    : "text-areia/55 hover:text-areia"
                }`}
              >
                Refine cut
              </button>
              <button
                type="button"
                onClick={() => setCutMode("generate")}
                title="Runs the full generation swarm and appends new clips to the timeline"
                className={`rounded-full px-2 py-0.5 font-medium transition-colors motion-reduce:transition-none ${
                  effectiveMode === "generate"
                    ? "bg-amarelo-ideias/25 text-amarelo-ideias"
                    : "text-areia/55 hover:text-areia"
                }`}
              >
                Fresh cut
              </button>
            </div>
            <span className="truncate text-[10px] text-areia/45">
              {effectiveMode === "refine" ? "Edits in place" : "Adds new cut"}
            </span>
          </div>

          {/* Context Pills (selected clips) */}
          {activeContextClips.length > 0 && (
            <div className="flex flex-wrap items-center gap-1">
              {activeContextClips.map(({ clip }) => (
                <span
                  key={clip.id}
                  className="inline-flex max-w-[140px] items-center gap-1 rounded-full border border-amarelo-ideias/35 bg-amarelo-ideias/10 px-2 py-0.5 text-[10px] text-areia"
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
          <div className="rounded-xl border border-areia/12 bg-void/75 p-2 focus-within:border-laranja-criacao">
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
                  ? "Describe what to adjust (Ctrl/Cmd+Enter to send)..."
                  : agentStatus?.openrouter_configured
                    ? `Direct swarm (${activeModelLabel ?? "OpenRouter"})...`
                    : "Direct swarm (Local mode)..."
              }
              className="w-full resize-none bg-transparent text-[11px] leading-snug text-areia placeholder:text-areia/35 outline-none disabled:opacity-50"
            />
            <div className="mt-1.5 flex items-center justify-between gap-2">
              <span className="text-[10px] text-areia/40">⌘/Ctrl+Enter</span>
              {isRunning ? (
                <button
                  type="button"
                  onClick={() => void onCancelSwarm()}
                  className="flex h-6 items-center gap-1 rounded-lg border border-vermelho-foco/60 bg-vermelho-foco/25 px-2.5 text-[10px] font-semibold text-areia hover:bg-vermelho-foco/40"
                >
                  <Square className="h-2.5 w-2.5 fill-current" />
                  <span>Cancel</span>
                </button>
              ) : (
                <button
                  type="button"
                  disabled={busy || !prompt.trim()}
                  onClick={() => void handleRun()}
                  className="flex h-6 items-center gap-1 rounded-lg bg-primary px-2.5 text-[10px] font-semibold text-areia transition-opacity disabled:opacity-40 motion-reduce:transition-none"
                >
                  <Play className="h-2.5 w-2.5 fill-current" />
                  <span>{effectiveMode === "refine" ? "Refine" : "Direct"}</span>
                </button>
              )}
            </div>
          </div>

          {/* Quick Refinement Chips */}
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
                className="rounded-full border border-areia/12 bg-void/60 px-2 py-0.5 text-[10px] text-areia/75 transition-colors hover:border-laranja-criacao/50 hover:bg-accent hover:text-areia disabled:opacity-40 motion-reduce:transition-none"
              >
                {chip.label}
              </button>
            ))}
          </div>


        </div>
      )}

      {/* TAB 2: ACTIVITY & QA */}
      {activeTab === "activity" && (
        <div className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto pr-1">
          {latestQaReport && (
            <div className="rounded-lg border border-areia/12 bg-void/70 px-2 py-1.5 text-[10px]">
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
                <div className="mt-1.5 max-h-40 space-y-1 overflow-y-auto pr-0.5">
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
                          className={`mt-0.5 shrink-0 rounded px-1 py-0.5 text-[10px] font-semibold uppercase leading-none ${
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

          <div className="space-y-1.5 text-[11px]">
            {activityLog.length === 0 ? (
              <div className="text-areia/40">Waiting for agent or user edits...</div>
            ) : (
              activityLog.map((entry) => (
                <div
                  key={entry.id}
                  className="flex items-start gap-2 rounded-lg bg-void/55 px-2 py-1.5"
                >
                  <span className="tabular-nums text-[10px] text-areia/40">
                    {entry.timecode}
                  </span>
                  <span className="shrink-0 rounded bg-grafite px-1.5 py-0.5 text-[10px] font-semibold text-amarelo-ideias">
                    {entry.agentId}
                  </span>
                  <span className="truncate text-areia/85">{entry.summary}</span>
                </div>
              ))
            )}
          </div>
        </div>
      )}

      {/* TAB 3: DIAGNOSTIC LOGS */}
      {activeTab === "logs" && (
        <div className="flex min-h-0 flex-1 flex-col">
          <div className="mb-1.5 flex items-center gap-1 text-[10px]">
            {(
              [
                ["all", "All"],
                ["error", "Errors"],
                ["warn", "Warn+"],
              ] as const
            ).map(([key, label]) => (
              <button
                key={key}
                type="button"
                onClick={() => setLogFilter(key)}
                className={`rounded-full px-2 py-0.5 font-medium ${
                  logFilter === key
                    ? "border border-laranja-criacao/50 bg-laranja-criacao/20 text-areia"
                    : "border border-areia/10 bg-void/50 text-areia/60 hover:text-areia"
                }`}
              >
                {label}
              </button>
            ))}
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto pr-1">
            <DiagnosticLogList
              compact
              logs={visibleLogs}
              emptyMessage="No diagnostic logs yet. Swarm, OpenRouter, QA, and export events will stream here."
            />
          </div>
        </div>
      )}
    </div>
  );
}
