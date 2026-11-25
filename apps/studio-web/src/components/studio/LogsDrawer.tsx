import { useEffect, useMemo, useRef, useState } from "react";
import { Check, Copy, Search, Terminal, Trash2, X } from "lucide-react";
import type { LogEntry } from "@solaris/engine-client";
import { DiagnosticLogList } from "./DiagnosticLogList";

interface LogsDrawerProps {
  open: boolean;
  logs: LogEntry[];
  onClear: () => Promise<void>;
  onClose: () => void;
}

type LevelFilter = "all" | "error" | "warn" | "info";

export function formatLogsForCopy(logs: LogEntry[]): string {
  return logs
    .map((entry) => {
      const header = `[${entry.timestamp}] [${entry.level.toUpperCase()}] [${entry.source}${
        entry.agent_id ? `/${entry.agent_id}` : ""
      }] ${entry.message}`;
      return entry.details ? `${header}\n${entry.details}` : header;
    })
    .join("\n\n");
}

export function LogsDrawer({ open, logs, onClear, onClose }: LogsDrawerProps) {
  const [filter, setFilter] = useState<LevelFilter>("all");
  const [query, setQuery] = useState("");
  const [copyStatus, setCopyStatus] = useState<"idle" | "copied" | "failed">("idle");
  const [clearing, setClearing] = useState(false);
  const [clearError, setClearError] = useState<string | null>(null);
  const copyTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    return () => {
      if (copyTimerRef.current) clearTimeout(copyTimerRef.current);
    };
  }, []);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return [...logs]
      .reverse()
      .filter((entry) => {
        if (filter === "error" && entry.level !== "error") return false;
        if (filter === "warn" && entry.level !== "warn" && entry.level !== "error") return false;
        if (filter === "info" && entry.level === "debug") return false;
        if (!q) return true;
        return (
          entry.message.toLowerCase().includes(q) ||
          entry.source.toLowerCase().includes(q) ||
          (entry.agent_id?.toLowerCase().includes(q) ?? false) ||
          (entry.details?.toLowerCase().includes(q) ?? false)
        );
      });
  }, [logs, filter, query]);

  if (!open) return null;

  const errorCount = logs.filter((l) => l.level === "error").length;
  const warnCount = logs.filter((l) => l.level === "warn").length;

  const handleCopyAll = () => {
    const text = formatLogsForCopy([...filtered].reverse());
    if (!text) return;
    const scheduleReset = () => {
      if (copyTimerRef.current) clearTimeout(copyTimerRef.current);
      copyTimerRef.current = setTimeout(() => setCopyStatus("idle"), 1600);
    };
    if (!navigator.clipboard?.writeText) {
      setCopyStatus("failed");
      scheduleReset();
      return;
    }
    void navigator.clipboard
      .writeText(text)
      .then(() => {
        setCopyStatus("copied");
        scheduleReset();
      })
      .catch(() => {
        setCopyStatus("failed");
        scheduleReset();
      });
  };

  const handleClear = async () => {
    if (clearing) return;
    setClearing(true);
    setClearError(null);
    try {
      await onClear();
    } catch (err) {
      setClearError(
        err instanceof Error ? err.message : "Failed to clear diagnostic logs",
      );
    } finally {
      setClearing(false);
    }
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="solaris-logs-drawer-title"
      onClick={onClose}
      className="fixed inset-0 z-50 flex items-center justify-center bg-void/80 p-4 backdrop-blur-md"
    >
      <div
        onClick={(e) => e.stopPropagation()}
        className="flex h-[82dvh] w-full max-w-4xl flex-col rounded-2xl border border-areia/15 bg-carbono/95 shadow-2xl"
      >
        <div className="flex items-center justify-between border-b border-areia/10 px-4 py-3">
          <div className="flex items-center gap-2.5">
            <Terminal className="h-4 w-4 text-amarelo-ideias" />
            <h2
              id="solaris-logs-drawer-title"
              className="text-xs font-semibold uppercase tracking-[0.14em] text-areia"
            >
              Diagnostic Logs
            </h2>
            <span className="tabular-nums rounded-full border border-areia/12 bg-void/70 px-2 py-0.5 text-[10px] text-areia/70">
              {logs.length} total
            </span>
            {errorCount > 0 && (
              <span className="tabular-nums rounded-full bg-vermelho-foco px-2 py-0.5 text-[10px] font-semibold text-areia">
                {errorCount} error{errorCount === 1 ? "" : "s"}
              </span>
            )}
            {warnCount > 0 && (
              <span className="tabular-nums rounded-full border border-laranja-criacao/40 bg-laranja-criacao/25 px-2 py-0.5 text-[10px] font-medium text-areia">
                {warnCount} warn
              </span>
            )}
          </div>
          <div className="flex items-center gap-1.5">
            <button
              type="button"
              disabled={filtered.length === 0}
              onClick={handleCopyAll}
              className="flex items-center gap-1 rounded-full border border-areia/15 bg-void/70 px-2.5 py-1 text-[10px] font-medium text-areia/85 hover:border-amarelo-ideias/50 hover:bg-accent disabled:opacity-40"
            >
              {copyStatus === "copied" ? (
                <Check className="h-3 w-3 text-amarelo-ideias" />
              ) : (
                <Copy className="h-3 w-3 text-amarelo-ideias" />
              )}
              <span>
                {copyStatus === "copied"
                  ? "Copied"
                  : copyStatus === "failed"
                    ? "Copy failed"
                    : "Copy Logs"}
              </span>
            </button>
            <button
              type="button"
              disabled={logs.length === 0 || clearing}
              onClick={() => void handleClear()}
              className="flex items-center gap-1 rounded-full border border-areia/15 bg-void/70 px-2.5 py-1 text-[10px] font-medium text-areia/80 hover:border-vermelho-foco/55 hover:bg-accent disabled:opacity-40"
            >
              <Trash2 className="h-3 w-3 text-laranja-criacao" />
              <span>Clear</span>
            </button>
            <button
              type="button"
              onClick={onClose}
              aria-label="Close logs"
              className="grid h-7 w-7 place-items-center rounded-full border border-areia/12 text-areia/75 hover:bg-accent hover:text-areia"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>

        <div className="flex flex-wrap items-center justify-between gap-2 border-b border-areia/10 bg-void/45 px-4 py-2">
          <div className="flex items-center gap-1">
            {(
              [
                ["all", "All"],
                ["error", "Errors"],
                ["warn", "Warn+"],
                ["info", "Info+"],
              ] as const
            ).map(([key, label]) => (
              <button
                key={key}
                type="button"
                onClick={() => setFilter(key)}
                className={`rounded-full px-2.5 py-1 text-[10px] font-medium transition-colors motion-reduce:transition-none ${
                  filter === key
                    ? "bg-laranja-criacao/25 text-areia border border-laranja-criacao/50"
                    : "border border-transparent text-areia/65 hover:text-areia"
                }`}
              >
                {label}
              </button>
            ))}
          </div>
          <div className="relative min-w-[200px] flex-1 max-w-xs">
            <Search className="pointer-events-none absolute top-1/2 left-2.5 h-3 w-3 -translate-y-1/2 text-areia/40" />
            <input
              type="text"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Filter model, tool, error, finish_reason..."
              className="h-7 w-full rounded-lg border border-areia/12 bg-void/80 pr-2.5 pl-7 text-[11px] text-areia placeholder:text-areia/35 outline-none focus:border-laranja-criacao"
            />
          </div>
        </div>

        {clearError && (
          <div className="mx-4 mt-2 rounded-lg border border-vermelho-foco/45 bg-vermelho-foco/15 px-3 py-1.5 text-[11px] text-areia">
            {clearError}
          </div>
        )}

        <div className="min-h-0 flex-1 overflow-y-auto p-4">
          <DiagnosticLogList
            logs={filtered}
            emptyMessage="No diagnostic logs match the current filter."
          />
        </div>
      </div>
    </div>
  );
}
