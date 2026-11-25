import { useEffect, useMemo, useRef, useState } from "react";
import {
  Check,
  ChevronDown,
  ChevronRight,
  Copy,
  Cpu,
  Maximize2,
  Play,
  ShieldCheck,
  Sparkles,
  Square,
  Terminal,
  Trash2,
} from "lucide-react";
import type {
  AgentRole,
  AgentStatusResponse,
  LogEntry,
  LogLevel,
  QaReport,
  SwarmRunStatus,
} from "@solaris/engine-client";
import { formatLogsForCopy } from "./LogsDrawer";

export interface ActivityEntry {
  id: string;
  agentId: string;
  role: AgentRole;
  summary: string;
  timecode: string;
}

interface SwarmActivityPanelProps {
  agentStatus: AgentStatusResponse | null;
  mcpEndpoint: string;
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
  onStartSwarm: (prompt: string) => Promise<void>;
  onCancelSwarm: () => Promise<void>;
  onRunQa: () => Promise<void>;
  onOpenAiSettings?: () => void;
}

function logBadgeTone(level: LogLevel): string {
  switch (level) {
    case "error":
      return "bg-vermelho-foco text-areia";
    case "warn":
      return "bg-laranja-criacao/30 text-areia";
    case "info":
      return "bg-amarelo-ideias/20 text-amarelo-ideias";
    default:
      return "bg-grafite text-areia/60";
  }
}

export function SwarmActivityPanel({
  agentStatus,
  mcpEndpoint,
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
  onOpenAiSettings,
}: SwarmActivityPanelProps) {
  const [prompt, setPrompt] = useState("");
  const [busy, setBusy] = useState(false);
  const [copiedMcp, setCopiedMcp] = useState(false);
  const [copiedLogs, setCopiedLogs] = useState(false);
  const [clearingLogs, setClearingLogs] = useState(false);
  const [activeTab, setActiveTab] = useState<"activity" | "logs">("activity");
  const [logFilter, setLogFilter] = useState<"all" | "error" | "warn">("all");
  const [expandedLogIds, setExpandedLogIds] = useState<Record<string, boolean>>({});
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const copyTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const copyLogsTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    return () => {
      if (copyTimerRef.current) clearTimeout(copyTimerRef.current);
      if (copyLogsTimerRef.current) clearTimeout(copyLogsTimerRef.current);
    };
  }, []);

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
      .catch(() => {
        setErrorMsg("Failed to copy logs to clipboard.");
      });
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

  const isRunning =
    activeSwarmRun !== null &&
    (activeSwarmRun.state === "queued" ||
      activeSwarmRun.state === "planning" ||
      activeSwarmRun.state === "executing_spine" ||
      activeSwarmRun.state === "executing_specialists" ||
      activeSwarmRun.state === "running_qa");

  const handleRun = async () => {
    if (busy || isRunning || !prompt.trim()) return;
    setBusy(true);
    setErrorMsg(null);
    try {
      await onStartSwarm(prompt.trim());
      setPrompt("");
    } catch (err) {
      setErrorMsg(err instanceof Error ? err.message : "Failed to start swarm");
    } finally {
      setBusy(false);
    }
  };

  const handleCopyMcp = () => {
    const markCopied = () => {
      setCopiedMcp(true);
      if (copyTimerRef.current) clearTimeout(copyTimerRef.current);
      copyTimerRef.current = setTimeout(() => setCopiedMcp(false), 1600);
    };
    if (navigator.clipboard?.writeText) {
      navigator.clipboard
        .writeText(mcpEndpoint)
        .then(markCopied)
        .catch(() => {
          setErrorMsg(`Copy blocked. MCP endpoint: ${mcpEndpoint}`);
        });
    } else {
      setErrorMsg(`Clipboard unavailable. MCP endpoint: ${mcpEndpoint}`);
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col rounded-2xl border border-areia/10 bg-carbono/70 p-3">
      <div className="mb-2 flex flex-wrap items-center justify-between gap-1.5 text-[10px] font-semibold uppercase tracking-[0.14em] text-areia/60">
        <div className="flex items-center gap-1.5">
          <Sparkles className="h-3 w-3 text-amarelo-ideias" />
          <span>Director &amp; Swarm</span>
        </div>
        <div className="flex flex-wrap items-center gap-1">
          {onOpenAiSettings && (
            <button
              type="button"
              onClick={onOpenAiSettings}
              title={activeModelLabel ? `AI Settings (${activeModelLabel})` : "OpenRouter & Model Advisor Settings"}
              className="flex items-center gap-1 rounded-full border border-areia/12 bg-void/65 px-2 py-0.5 text-[10px] font-medium normal-case tracking-normal text-areia/80 hover:border-amarelo-ideias/50 hover:bg-accent"
            >
              <Cpu className="h-2.5 w-2.5 text-amarelo-ideias" />
              <span>AI</span>
            </button>
          )}
          <button
            type="button"
            disabled={qaRunning}
            onClick={() => void onRunQa()}
            title="Run Stage-3 Playback QA (frames, -14 LUFS & safe zones)"
            className="flex items-center gap-1 rounded-full border border-areia/12 bg-void/65 px-2 py-0.5 text-[10px] font-medium normal-case tracking-normal text-areia/80 hover:border-amarelo-ideias/50 hover:bg-accent disabled:opacity-45"
          >
            <ShieldCheck className="h-2.5 w-2.5 text-amarelo-ideias" />
            <span>{qaRunning ? "QA..." : "QA"}</span>
          </button>
          <button
            type="button"
            onClick={handleCopyMcp}
            title="Copy Streamable HTTP MCP endpoint (/mcp)"
            className="flex items-center gap-1 rounded-full border border-areia/12 bg-void/65 px-2 py-0.5 text-[10px] font-medium normal-case tracking-normal text-areia/75 hover:border-laranja-criacao/50 hover:bg-accent"
          >
            {copiedMcp ? (
              <Check className="h-2.5 w-2.5 text-amarelo-ideias" />
            ) : (
              <Copy className="h-2.5 w-2.5 text-laranja-criacao" />
            )}
            <span>MCP</span>
          </button>
          <span className="tabular-nums text-amarelo-ideias">
            {activityLog.length} ops
          </span>
        </div>
      </div>

      {/* Inline Director Prompt Bar */}
      <div className="mb-2 flex items-center gap-1.5">
        <input
          type="text"
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !isRunning && !busy) {
              void handleRun();
            }
          }}
          placeholder={
            agentStatus?.openrouter_configured
              ? `Direct swarm (${activeModelLabel ?? "OpenRouter"})...`
              : "Direct swarm (Local mode)..."
          }
          className="h-7 min-w-0 flex-1 rounded-lg border border-areia/12 bg-void/75 px-2 text-[11px] text-areia placeholder:text-areia/35 outline-none focus:border-laranja-criacao"
        />
        {isRunning ? (
          <button
            type="button"
            onClick={() => void onCancelSwarm()}
            className="flex h-7 shrink-0 items-center gap-1 rounded-lg border border-vermelho-foco/60 bg-vermelho-foco/25 px-2.5 text-[10px] font-semibold text-areia hover:bg-vermelho-foco/40"
          >
            <Square className="h-2.5 w-2.5 fill-current" />
            <span>Stop</span>
          </button>
        ) : (
          <button
            type="button"
            disabled={busy || !prompt.trim()}
            onClick={() => void handleRun()}
            className="flex h-7 shrink-0 items-center gap-1 rounded-lg bg-primary px-2.5 text-[10px] font-semibold text-areia transition-opacity disabled:opacity-40 motion-reduce:transition-none"
          >
            <Play className="h-2.5 w-2.5 fill-current" />
            <span>Direct</span>
          </button>
        )}
      </div>

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

      {latestQaReport && (
        <div className="mb-2 rounded-lg border border-areia/12 bg-void/70 px-2 py-1.5 text-[10px]">
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
            <div className="mt-1.5 max-h-48 space-y-1 overflow-y-auto pr-0.5">
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

      {/* Activity / Diagnostic Logs Segmented Bar */}
      <div className="mb-2 flex items-center justify-between gap-1.5 border-y border-areia/10 py-1.5">
        <div
          role="tablist"
          aria-label="Swarm feed mode"
          className="flex items-center gap-0.5 rounded-full border border-areia/10 bg-void/65 p-0.5 text-[10px]"
        >
          <button
            type="button"
            role="tab"
            aria-selected={activeTab === "activity"}
            onClick={() => setActiveTab("activity")}
            className={`rounded-full px-2 py-0.5 font-medium transition-colors motion-reduce:transition-none ${
              activeTab === "activity"
                ? "bg-laranja-criacao/25 text-areia"
                : "text-areia/55 hover:text-areia"
            }`}
          >
            Activity
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={activeTab === "logs"}
            onClick={() => setActiveTab("logs")}
            className={`flex items-center gap-1 rounded-full px-2 py-0.5 font-medium transition-colors motion-reduce:transition-none ${
              activeTab === "logs"
                ? "bg-laranja-criacao/25 text-areia"
                : "text-areia/55 hover:text-areia"
            }`}
          >
            <span>Logs</span>
            <span className="tabular-nums text-areia/65">{logs.length}</span>
            {errorLogCount > 0 && (
              <span className="tabular-nums rounded-full bg-vermelho-foco px-1.5 text-[10px] font-semibold text-areia">
                {errorLogCount}
              </span>
            )}
          </button>
        </div>

        <div className="flex items-center gap-1">
          {activeTab === "logs" && (
            <>
              <button
                type="button"
                disabled={visibleLogs.length === 0}
                onClick={handleCopyLogs}
                title="Copy visible logs"
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

      {activeTab === "logs" && (
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
      )}

      <div className="flex-1 space-y-1.5 overflow-y-auto pr-1 text-[11px]">
        {activeTab === "activity" ? (
          activityLog.length === 0 ? (
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
          )
        ) : visibleLogs.length === 0 ? (
          <div className="text-areia/40">
            No diagnostic logs yet. Swarm, OpenRouter, QA, and export events will stream here.
          </div>
        ) : (
          visibleLogs.map((entry) => {
            const hasDetails = Boolean(entry.details);
            const expanded =
              expandedLogIds[entry.id] ?? (entry.level === "error" && hasDetails);
            return (
              <div
                key={entry.id}
                className={`rounded-lg border px-2 py-1.5 text-[10px] ${
                  entry.level === "error"
                    ? "border-vermelho-foco/45 bg-vermelho-foco/12"
                    : entry.level === "warn"
                      ? "border-laranja-criacao/35 bg-laranja-criacao/10"
                      : "border-areia/10 bg-void/60"
                }`}
              >
                <button
                  type="button"
                  disabled={!hasDetails}
                  onClick={() =>
                    hasDetails &&
                    setExpandedLogIds((prev) => ({ ...prev, [entry.id]: !expanded }))
                  }
                  className="flex w-full items-start gap-1.5 text-left disabled:cursor-default"
                >
                  {hasDetails && (
                    <span className="mt-0.5 shrink-0 text-areia/50">
                      {expanded ? (
                        <ChevronDown className="h-2.5 w-2.5" />
                      ) : (
                        <ChevronRight className="h-2.5 w-2.5" />
                      )}
                    </span>
                  )}
                  <span className="tabular-nums shrink-0 text-areia/45">
                    {entry.timestamp.slice(11, 19)}
                  </span>
                  <span
                    className={`shrink-0 rounded px-1 py-0.5 font-semibold uppercase ${logBadgeTone(
                      entry.level,
                    )}`}
                  >
                    {entry.level === "error"
                      ? "ERR"
                      : entry.level === "warn"
                        ? "WRN"
                        : entry.level === "info"
                          ? "INF"
                          : "DBG"}
                  </span>
                  <span className="min-w-0 flex-1 break-words text-areia/85">
                    {entry.agent_id && (
                      <span className="mr-1 font-semibold text-amarelo-ideias">
                        [{entry.agent_id}]
                      </span>
                    )}
                    {entry.message}
                  </span>
                </button>
                {hasDetails && expanded && (
                  <pre className="mt-1.5 max-h-40 overflow-auto whitespace-pre-wrap break-all rounded border border-areia/10 bg-void/90 p-1.5 text-[10px] leading-snug text-areia/80">
                    {entry.details}
                  </pre>
                )}
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
