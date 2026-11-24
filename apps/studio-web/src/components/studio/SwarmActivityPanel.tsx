import { useEffect, useRef, useState } from "react";
import { Check, Copy, Cpu, Play, ShieldCheck, Square, Sparkles } from "lucide-react";
import type {
  AgentRole,
  AgentStatusResponse,
  QaReport,
  SwarmRunStatus,
} from "@solaris/engine-client";

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
  onStartSwarm: (prompt: string) => Promise<void>;
  onCancelSwarm: () => Promise<void>;
  onRunQa: () => Promise<void>;
  onOpenAiSettings?: () => void;
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
  onStartSwarm,
  onCancelSwarm,
  onRunQa,
  onOpenAiSettings,
}: SwarmActivityPanelProps) {
  const [prompt, setPrompt] = useState("");
  const [busy, setBusy] = useState(false);
  const [copiedMcp, setCopiedMcp] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const copyTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    return () => {
      if (copyTimerRef.current) clearTimeout(copyTimerRef.current);
    };
  }, []);

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
          setErrorMsg(`Copy blocked — MCP endpoint: ${mcpEndpoint}`);
        });
    } else {
      setErrorMsg(`Clipboard unavailable — MCP endpoint: ${mcpEndpoint}`);
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col rounded-2xl border border-areia/10 bg-carbono/70 p-3">
      <div className="mb-2 flex items-center justify-between text-[10px] font-semibold uppercase tracking-[0.14em] text-areia/60">
        <div className="flex items-center gap-1.5">
          <Sparkles className="h-3 w-3 text-amarelo-ideias" />
          <span>Director &amp; Swarm</span>
        </div>
        <div className="flex items-center gap-1.5">
          {onOpenAiSettings && (
            <button
              type="button"
              onClick={onOpenAiSettings}
              title={activeModelLabel ? `AI Settings (${activeModelLabel})` : "OpenRouter & Model Advisor Settings"}
              className="flex items-center gap-1 rounded-full border border-areia/12 bg-void/65 px-2 py-0.5 text-[9px] font-medium normal-case tracking-normal text-areia/80 hover:border-amarelo-ideias/50 hover:bg-accent"
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
            className="flex items-center gap-1 rounded-full border border-areia/12 bg-void/65 px-2 py-0.5 text-[9px] font-medium normal-case tracking-normal text-areia/80 hover:border-amarelo-ideias/50 hover:bg-accent disabled:opacity-45"
          >
            <ShieldCheck className="h-2.5 w-2.5 text-amarelo-ideias" />
            <span>{qaRunning ? "QA..." : "QA"}</span>
          </button>
          <button
            type="button"
            onClick={handleCopyMcp}
            title="Copy Streamable HTTP MCP endpoint (/mcp)"
            className="flex items-center gap-1 rounded-full border border-areia/12 bg-void/65 px-2 py-0.5 text-[9px] font-medium normal-case tracking-normal text-areia/75 hover:border-laranja-criacao/50 hover:bg-accent"
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
            <div className="mt-1 rounded border border-vermelho-foco/40 bg-vermelho-foco/15 px-1.5 py-0.5 text-areia">
              {activeSwarmRun.error}
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
            <p className="mt-1 line-clamp-2 text-[9px] text-areia/75">
              {latestQaReport.multimodal_summary}
            </p>
          )}
          {latestQaReport.checks.length > 0 && (
            <div className="mt-1 max-h-24 space-y-0.5 overflow-y-auto pr-0.5">
              {[...latestQaReport.checks]
                .sort((a, b) => {
                  const rank = { issue: 0, warning: 1, fixed: 2, pass: 3 };
                  return rank[a.severity] - rank[b.severity];
                })
                .map((chk) => (
                  <div
                    key={chk.id}
                    className="flex items-center gap-1.5 text-[9px] text-areia/80"
                  >
                    <span
                      className={`shrink-0 rounded px-1 py-0.25 text-[8px] font-semibold uppercase ${
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
                    <span className="truncate">{chk.message}</span>
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

      <div className="flex-1 space-y-1.5 overflow-y-auto pr-1 text-[11px]">
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
              <span className="shrink-0 rounded bg-grafite px-1.5 py-0.5 text-[9px] font-semibold text-amarelo-ideias">
                {entry.agentId}
              </span>
              <span className="truncate text-areia/85">{entry.summary}</span>
            </div>
          ))
        )}
      </div>
    </div>
  );
}
