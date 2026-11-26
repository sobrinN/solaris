import { useMemo, useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import type { LogEntry, LogLevel } from "@solaris/engine-client";

interface DiagnosticLogListProps {
  logs: LogEntry[];
  emptyMessage: string;
}

function levelBadgeClass(level: LogLevel): string {
  switch (level) {
    case "error":
      return "bg-vermelho-foco text-areia";
    case "warn":
      return "bg-laranja-criacao/30 text-areia border border-laranja-criacao/40";
    case "info":
      return "bg-amarelo-ideias/20 text-amarelo-ideias";
    default:
      return "bg-grafite text-areia/65";
  }
}

export function DiagnosticLogList({
  logs,
  emptyMessage,
}: DiagnosticLogListProps) {
  const [expandedIds, setExpandedIds] = useState<Record<string, boolean>>({});

  const autoExpandedErrorIds = useMemo(() => {
    const ids = new Set<string>();
    for (const entry of logs) {
      if (entry.level === "error" && entry.details) {
        ids.add(entry.id);
        if (ids.size >= 10) break;
      }
    }
    return ids;
  }, [logs]);

  if (logs.length === 0) {
    return (
      <div className="rounded-xl border border-dashed border-areia/12 py-12 text-center text-xs text-areia/45">
        {emptyMessage}
      </div>
    );
  }

  return (
    <div className="space-y-1.5">
      {logs.map((entry) => {
        const hasDetails = Boolean(entry.details);
        const isExpanded = expandedIds[entry.id] ?? autoExpandedErrorIds.has(entry.id);
        return (
          <div
            key={entry.id}
            className={`rounded-xl border px-3 py-2 text-[11px] ${
              entry.level === "error"
                ? "border-vermelho-foco/45 bg-vermelho-foco/12"
                : entry.level === "warn"
                  ? "border-laranja-criacao/35 bg-laranja-criacao/10"
                  : "border-areia/10 bg-void/65"
            }`}
          >
            <button
              type="button"
              disabled={!hasDetails}
              onClick={() =>
                hasDetails &&
                setExpandedIds((prev) => ({ ...prev, [entry.id]: !isExpanded }))
              }
              className="flex w-full items-start gap-2 text-left disabled:cursor-default"
            >
              {hasDetails ? (
                <span className="mt-0.5 shrink-0 text-areia/45">
                  {isExpanded ? (
                    <ChevronDown className="h-3 w-3" />
                  ) : (
                    <ChevronRight className="h-3 w-3" />
                  )}
                </span>
              ) : (
                <span className="inline-block h-3 w-3 shrink-0" />
              )}
              <span className="tabular-nums shrink-0 text-[10px] text-areia/50">
                {entry.timestamp.slice(11, 19)}
              </span>
              <span
                className={`shrink-0 rounded px-1.5 py-0.5 text-[10px] font-semibold uppercase ${levelBadgeClass(entry.level)}`}
              >
                {entry.level}
              </span>
              <span className="shrink-0 rounded bg-grafite px-1.5 py-0.5 text-[10px] font-medium text-amarelo-ideias">
                {entry.agent_id ? `${entry.source}/${entry.agent_id}` : entry.source}
              </span>
              <span className="min-w-0 flex-1 break-words text-areia/90">
                {entry.message}
              </span>
            </button>
            {hasDetails && isExpanded && (
              <pre className="mt-1.5 max-h-64 overflow-auto whitespace-pre-wrap break-all rounded-lg border border-areia/10 bg-void/90 p-2.5 text-[10px] leading-relaxed text-areia/80">
                {entry.details}
              </pre>
            )}
          </div>
        );
      })}
    </div>
  );
}
