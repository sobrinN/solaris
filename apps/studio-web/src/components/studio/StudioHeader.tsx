import {
  Cpu,
  Download,
  Palette,
  Plus,
  Redo2,
  Terminal,
  Undo2,
  Users,
} from "lucide-react";
import type {
  AgentRole,
  AgentStatusResponse,
  AspectPreset,
  ExportJobStatus,
  LogEntry,
  Project,
  ProjectSummary,
} from "@solaris/engine-client";
import { SolarisWordmark } from "../SolarisBrand";
import { ProjectManagerPopover } from "./ProjectManagerPopover";

interface StudioHeaderProps {
  project: Project;
  projects: ProjectSummary[];
  projectManagerOpen: boolean;
  onProjectManagerOpenChange: (open: boolean) => void;
  onSelectProject: (projectId: string) => Promise<void>;
  onCreateBlankProject: (name: string, aspect: AspectPreset) => Promise<void>;
  onRenameProject: (name: string) => Promise<void>;
  onDeleteProject: (projectId: string) => Promise<void>;
  onBackToChat: () => void;
  onUndo: () => Promise<void>;
  onRedo: () => Promise<void>;
  wsConnected: boolean;
  swarmCursorsEnabled: boolean;
  onToggleSwarmCursors: () => void;
  isSwarmActive: boolean;
  activeRoles: AgentRole[];
  logs: LogEntry[];
  onOpenLogsDrawer: () => void;
  agentStatus: AgentStatusResponse | null;
  onOpenAiSettings: () => void;
  onOpenBrandDrawer: () => void;
  exportJob: ExportJobStatus | null;
  onOpenExportModal: () => void;
}

const PRESENCE_DOTS: Array<{ role: AgentRole; colorClass: string; label: string }> = [
  { role: "director", colorClass: "bg-amarelo-ideias", label: "Director" },
  { role: "gfx", colorClass: "bg-track-g1", label: "GFX" },
  { role: "b_roll", colorClass: "bg-track-v2", label: "B-Roll" },
  { role: "caps", colorClass: "bg-track-a1", label: "Audio/Caps" },
];

export function StudioHeader({
  project,
  projects,
  projectManagerOpen,
  onProjectManagerOpenChange,
  onSelectProject,
  onCreateBlankProject,
  onRenameProject,
  onDeleteProject,
  onBackToChat,
  onUndo,
  onRedo,
  wsConnected,
  swarmCursorsEnabled,
  onToggleSwarmCursors,
  isSwarmActive,
  activeRoles,
  logs,
  onOpenLogsDrawer,
  agentStatus,
  onOpenAiSettings,
  onOpenBrandDrawer,
  exportJob,
  onOpenExportModal,
}: StudioHeaderProps) {
  const activeRoleSet = new Set(activeRoles);
  const errorCount = logs.filter((l) => l.level === "error").length;
  const exportInFlight =
    exportJob &&
    (exportJob.state === "queued" ||
      exportJob.state === "rendering_gfx" ||
      exportJob.state === "compositing");

  return (
    <header className="relative z-30 flex h-11 shrink-0 items-center justify-between gap-2.5 border-b border-areia/[0.09] bg-carbono/85 px-3 shadow-[inset_0_1px_0_rgba(247,245,239,0.04)] backdrop-blur-md">
      {/* Left Zone: Brand + Project Manager */}
      <div className="flex min-w-0 items-center gap-2">
        <SolarisWordmark />
        <div className="hidden h-3.5 w-px bg-areia/15 sm:block" />
        <ProjectManagerPopover
          currentProject={project}
          projects={projects}
          open={projectManagerOpen}
          onOpenChange={onProjectManagerOpenChange}
          onSelectProject={onSelectProject}
          onCreateBlankProject={onCreateBlankProject}
          onRenameProject={onRenameProject}
          onDeleteProject={onDeleteProject}
          onNewProjectInChat={onBackToChat}
        />
        <button
          type="button"
          onClick={onBackToChat}
          title="Start a new project with fresh media or prompt"
          className="hidden h-7 items-center gap-1 rounded-full border border-areia/10 bg-void/70 px-2.5 text-[11px] text-areia/80 transition-colors hover:border-laranja-criacao/50 hover:bg-accent hover:text-areia md:flex motion-reduce:transition-none"
        >
          <Plus className="h-3 w-3 text-laranja-criacao" />
          <span>New</span>
        </button>
      </div>

      {/* Center Zone: History & Multi-Agent Swarm Presence */}
      <div className="hidden items-center gap-2 lg:flex">
        <div className="flex items-center rounded-full border border-areia/10 bg-void/70 p-0.5">
          <button
            type="button"
            onClick={() => void onUndo()}
            className="grid h-6 w-6 place-items-center rounded-full text-areia/80 transition-colors hover:bg-accent hover:text-areia motion-reduce:transition-none"
            title="Undo (⌘Z)"
            aria-label="Undo"
          >
            <Undo2 className="h-3 w-3" />
          </button>
          <button
            type="button"
            onClick={() => void onRedo()}
            className="grid h-6 w-6 place-items-center rounded-full text-areia/80 transition-colors hover:bg-accent hover:text-areia motion-reduce:transition-none"
            title="Redo (⇧⌘Z)"
            aria-label="Redo"
          >
            <Redo2 className="h-3 w-3" />
          </button>
        </div>

        <button
          type="button"
          onClick={onToggleSwarmCursors}
          title={
            wsConnected
              ? "Swarm WebSocket connected · Toggle agent cursors"
              : "Reconnecting to Solaris Engine"
          }
          className={`flex h-7 items-center gap-2 rounded-full border px-2.5 text-[11px] transition-colors motion-reduce:transition-none ${
            swarmCursorsEnabled
              ? "border-laranja-criacao/55 bg-accent text-areia"
              : "border-areia/10 bg-void/70 text-areia/75 hover:bg-accent hover:text-areia"
          }`}
        >
          <span
            className={`h-1.5 w-1.5 rounded-full ${
              wsConnected ? "bg-amarelo-ideias solar-pulse-dot" : "bg-vermelho-foco"
            }`}
          />
          {swarmCursorsEnabled ? (
            <span className="flex items-center -space-x-1" aria-hidden="true">
              {PRESENCE_DOTS.map((dot) => {
                const active = isSwarmActive && activeRoleSet.has(dot.role);
                return (
                  <span
                    key={dot.role}
                    className={`h-2 w-2 rounded-full ring-1 ring-void ${dot.colorClass} ${
                      active ? "solar-pulse-dot" : "opacity-85"
                    }`}
                  />
                );
              })}
            </span>
          ) : (
            <Users className="h-3 w-3 text-laranja-criacao" />
          )}
          <span className="font-medium">
            {swarmCursorsEnabled ? "Swarm Live" : "Focus Mode"}
          </span>
        </button>
      </div>

      {/* Right Zone: Studio Utilities + Primary Export CTA */}
      <div className="flex items-center gap-1.5">
        <div className="flex items-center gap-1">
          <button
            type="button"
            onClick={onOpenLogsDrawer}
            className="flex h-7 items-center gap-1.5 rounded-full border border-areia/10 bg-void/60 px-2.5 text-[11px] text-areia/85 transition-colors hover:bg-accent motion-reduce:transition-none"
            title="Open Diagnostic Logs (OpenRouter, Swarm, QA & Export)"
            aria-label="Diagnostic Logs"
          >
            <Terminal className="h-3 w-3 text-amarelo-ideias" />
            <span className="hidden xl:inline">Logs</span>
            {errorCount > 0 && (
              <span className="tabular-nums rounded-full bg-vermelho-foco px-1.5 text-[9px] font-semibold text-areia">
                {errorCount}
              </span>
            )}
          </button>

          <button
            type="button"
            onClick={onOpenAiSettings}
            className="flex h-7 items-center gap-1.5 rounded-full border border-areia/10 bg-void/60 px-2.5 text-[11px] text-areia/85 transition-colors hover:bg-accent motion-reduce:transition-none"
            title="OpenRouter API Key, Model Advisor & MCP Mode"
            aria-label="AI Settings"
          >
            <Cpu className="h-3 w-3 text-amarelo-ideias" />
            <span className="hidden xl:inline">
              {agentStatus?.openrouter_configured ? "AI Ready" : "AI Keys"}
            </span>
          </button>

          <button
            type="button"
            data-cursor-anchor="brand.playbook"
            onClick={onOpenBrandDrawer}
            className="flex h-7 items-center gap-1.5 rounded-full border border-areia/10 bg-void/60 px-2.5 text-[11px] text-areia/85 transition-colors hover:bg-accent motion-reduce:transition-none"
            title="Open Brand Playbook"
            aria-label="Brand Playbook"
          >
            <Palette className="h-3 w-3 text-laranja-criacao" />
            <span className="hidden md:inline">Brand</span>
          </button>
        </div>

        <div className="mx-0.5 hidden h-3.5 w-px bg-areia/14 sm:block" />

        <button
          type="button"
          onClick={onOpenExportModal}
          className="btn-solar-primary flex h-7 items-center gap-1.5 px-3.5 text-[11px] font-semibold"
        >
          <Download className="h-3 w-3 text-areia" />
          <span className="tabular-nums">
            {exportInFlight
              ? `Export · ${Math.round(exportJob.progress * 100)}%`
              : "Export"}
          </span>
        </button>
      </div>
    </header>
  );
}
