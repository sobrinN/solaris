import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import {
  Check,
  ChevronDown,
  FolderKanban,
  Pencil,
  Plus,
  Search,
  Sparkles,
  X,
} from "lucide-react";
import type { AspectPreset, Project, ProjectSummary } from "@solaris/engine-client";

interface ProjectManagerPopoverProps {
  currentProject: Project;
  projects: ProjectSummary[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSelectProject: (projectId: string) => Promise<void>;
  onCreateBlankProject: (name: string, aspect: AspectPreset) => Promise<void>;
  onRenameProject: (name: string) => Promise<void>;
  onNewProjectInChat: () => void;
}

const ASPECT_LABELS: Record<AspectPreset, string> = {
  vertical9x16: "9:16",
  landscape16x9: "16:9",
  square1x1: "1:1",
};

const ASPECT_OPTIONS: Array<{ value: AspectPreset; label: string }> = [
  { value: "vertical9x16", label: "9:16" },
  { value: "landscape16x9", label: "16:9" },
  { value: "square1x1", label: "1:1" },
];

function formatRelativeTime(iso: string): string {
  const ts = Date.parse(iso);
  if (!Number.isFinite(ts)) return "";
  const diffSec = Math.max(0, Math.floor((Date.now() - ts) / 1000));
  if (diffSec < 60) return "Just now";
  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return `${diffMin}m ago`;
  const diffHr = Math.floor(diffMin / 60);
  if (diffHr < 24) return `${diffHr}h ago`;
  const diffDays = Math.floor(diffHr / 24);
  if (diffDays < 7) return `${diffDays}d ago`;
  return new Date(ts).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function formatDuration(sec: number): string {
  if (!Number.isFinite(sec) || sec <= 0) return "0s";
  if (sec < 60) return `${Math.round(sec)}s`;
  const mins = Math.floor(sec / 60);
  const rem = Math.round(sec % 60);
  return rem > 0 ? `${mins}m ${rem}s` : `${mins}m`;
}

export function ProjectManagerPopover({
  currentProject,
  projects,
  open,
  onOpenChange,
  onSelectProject,
  onCreateBlankProject,
  onRenameProject,
  onNewProjectInChat,
}: ProjectManagerPopoverProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [search, setSearch] = useState("");
  const [renaming, setRenaming] = useState(false);
  const [renameDraft, setRenameDraft] = useState(currentProject.name);
  const [creatingBlank, setCreatingBlank] = useState(false);
  const [newName, setNewName] = useState("");
  const [newAspect, setNewAspect] = useState<AspectPreset>("vertical9x16");
  const [busyId, setBusyId] = useState<string | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  useEffect(() => {
    setRenameDraft(currentProject.name);
    setRenaming(false);
  }, [currentProject.id, currentProject.name]);

  useEffect(() => {
    if (!open) {
      setSearch("");
      setRenaming(false);
      setCreatingBlank(false);
      setErrorMsg(null);
      return;
    }
    const onPointerDown = (e: PointerEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        onOpenChange(false);
      }
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (renaming) {
          setRenaming(false);
          setRenameDraft(currentProject.name);
        } else if (creatingBlank) {
          setCreatingBlank(false);
        } else {
          onOpenChange(false);
        }
      }
    };
    window.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [open, renaming, creatingBlank, currentProject.name, onOpenChange]);

  const sortedProjects = useMemo(() => {
    const q = search.trim().toLowerCase();
    const list = [...projects].sort((a, b) => b.updated_at.localeCompare(a.updated_at));
    if (!q) return list;
    return list.filter(
      (p) =>
        p.name.toLowerCase().includes(q) ||
        ASPECT_LABELS[p.aspect_preset].toLowerCase().includes(q),
    );
  }, [projects, search]);

  const handleRenameSubmit = async (e: FormEvent) => {
    e.preventDefault();
    const trimmed = renameDraft.trim();
    if (!trimmed || trimmed === currentProject.name) {
      setRenaming(false);
      return;
    }
    setBusyId("rename");
    setErrorMsg(null);
    try {
      await onRenameProject(trimmed);
      setRenaming(false);
    } catch (err) {
      setErrorMsg(err instanceof Error ? err.message : "Failed to rename project");
    } finally {
      setBusyId(null);
    }
  };

  const handleCreateBlankSubmit = async (e: FormEvent) => {
    e.preventDefault();
    const trimmed = newName.trim() || `project_${projects.length + 1}`;
    setBusyId("create");
    setErrorMsg(null);
    try {
      await onCreateBlankProject(trimmed, newAspect);
      setNewName("");
      setCreatingBlank(false);
      onOpenChange(false);
    } catch (err) {
      setErrorMsg(err instanceof Error ? err.message : "Failed to create project");
    } finally {
      setBusyId(null);
    }
  };

  const handleSwitchProject = async (projectId: string) => {
    if (projectId === currentProject.id) {
      onOpenChange(false);
      return;
    }
    setBusyId(projectId);
    setErrorMsg(null);
    try {
      await onSelectProject(projectId);
      onOpenChange(false);
    } catch (err) {
      setErrorMsg(err instanceof Error ? err.message : "Failed to switch project");
    } finally {
      setBusyId(null);
    }
  };

  const currentAspectLabel = ASPECT_LABELS[currentProject.canvas.aspect_preset];

  return (
    <div ref={containerRef} className="relative">
      <button
        type="button"
        onClick={() => onOpenChange(!open)}
        aria-expanded={open}
        aria-haspopup="dialog"
        title={`Switch or manage projects (${currentProject.name})`}
        className={`flex h-8 max-w-[280px] items-center gap-2 rounded-full border px-3 text-xs transition-colors motion-reduce:transition-none ${
          open
            ? "border-laranja-criacao/65 bg-accent text-areia"
            : "border-areia/14 bg-void/70 text-areia/90 hover:border-areia/28 hover:bg-accent"
        }`}
      >
        <FolderKanban className="h-3.5 w-3.5 shrink-0 text-laranja-criacao" />
        <span className="truncate font-medium">{currentProject.name}</span>
        <span className="hidden shrink-0 rounded bg-grafite/90 px-1.5 py-0.5 text-[10px] font-medium tabular-nums text-amarelo-ideias sm:inline">
          {currentAspectLabel}
        </span>
        <span className="hidden shrink-0 tabular-nums text-[10px] text-areia/50 md:inline">
          r{currentProject.revision}
        </span>
        <ChevronDown
          className={`h-3.5 w-3.5 shrink-0 text-areia/55 transition-transform duration-150 motion-reduce:transition-none ${
            open ? "rotate-180 text-areia" : ""
          }`}
        />
      </button>

      {open && (
        <div
          role="dialog"
          aria-label="Project Manager"
          className="absolute left-0 z-50 mt-2 w-[340px] rounded-2xl border border-areia/14 bg-carbono/96 p-3 shadow-[0_24px_64px_-16px_rgba(0,0,0,0.92)] backdrop-blur-xl sm:w-[376px]"
        >
          {/* Active Project Header & Inline Rename */}
          <div className="rounded-xl border border-areia/10 bg-void/75 p-2.5">
            <div className="mb-1.5 flex items-center justify-between text-[10px] font-semibold uppercase tracking-[0.14em] text-areia/50">
              <span>Current Project</span>
              <span className="tabular-nums text-amarelo-ideias">
                {currentAspectLabel} · rev {currentProject.revision}
              </span>
            </div>

            {renaming ? (
              <form onSubmit={(e) => void handleRenameSubmit(e)} className="flex items-center gap-1.5">
                <input
                  type="text"
                  autoFocus
                  value={renameDraft}
                  onChange={(e) => setRenameDraft(e.target.value)}
                  disabled={busyId === "rename"}
                  className="min-w-0 flex-1 rounded-lg border border-laranja-criacao/60 bg-carbono px-2.5 py-1 text-xs text-areia outline-none"
                  placeholder="Project name"
                />
                <button
                  type="submit"
                  disabled={busyId === "rename" || !renameDraft.trim()}
                  className="grid h-7 w-7 shrink-0 place-items-center rounded-lg bg-primary text-areia disabled:opacity-40"
                  aria-label="Save project name"
                >
                  <Check className="h-3.5 w-3.5" />
                </button>
                <button
                  type="button"
                  onClick={() => {
                    setRenaming(false);
                    setRenameDraft(currentProject.name);
                  }}
                  className="grid h-7 w-7 shrink-0 place-items-center rounded-lg border border-areia/12 text-areia/65 hover:text-areia"
                  aria-label="Cancel rename"
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              </form>
            ) : (
              <div className="flex items-center justify-between gap-2">
                <span className="truncate text-xs font-medium text-areia" title={currentProject.name}>
                  {currentProject.name}
                </span>
                <button
                  type="button"
                  onClick={() => {
                    setRenameDraft(currentProject.name);
                    setRenaming(true);
                  }}
                  className="flex shrink-0 items-center gap-1 rounded-full border border-areia/12 px-2 py-0.5 text-[10px] text-areia/75 hover:border-laranja-criacao/50 hover:bg-accent hover:text-areia"
                >
                  <Pencil className="h-2.5 w-2.5 text-amarelo-ideias" />
                  <span>Rename</span>
                </button>
              </div>
            )}
          </div>

          {/* Primary Actions: New Project with Media/Prompt or Quick Blank */}
          <div className="mt-2.5 flex items-center gap-2">
            <button
              type="button"
              onClick={() => {
                onOpenChange(false);
                onNewProjectInChat();
              }}
              className="btn-solar-outline flex h-8 flex-1 items-center justify-center gap-1.5 px-3 text-xs font-medium text-areia"
            >
              <Sparkles className="h-3.5 w-3.5 text-amarelo-ideias" />
              <span>New Project</span>
            </button>
            <button
              type="button"
              onClick={() => setCreatingBlank((v) => !v)}
              className={`flex h-8 items-center gap-1 rounded-full border px-3 text-xs transition-colors motion-reduce:transition-none ${
                creatingBlank
                  ? "border-laranja-criacao/60 bg-accent text-areia"
                  : "border-areia/12 bg-void/65 text-areia/80 hover:bg-accent hover:text-areia"
              }`}
            >
              <Plus className="h-3.5 w-3.5 text-laranja-criacao" />
              <span>Blank</span>
            </button>
          </div>

          {/* Inline Quick Blank Project Creator */}
          {creatingBlank && (
            <form
              onSubmit={(e) => void handleCreateBlankSubmit(e)}
              className="mt-2.5 space-y-2 rounded-xl border border-laranja-criacao/30 bg-void/80 p-2.5"
            >
              <input
                type="text"
                autoFocus
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
                placeholder="Untitled project name..."
                className="w-full rounded-lg border border-areia/12 bg-carbono px-2.5 py-1.5 text-xs text-areia placeholder:text-areia/35 outline-none focus:border-laranja-criacao"
              />
              <div className="flex items-center justify-between gap-2">
                <div className="flex items-center gap-1">
                  {ASPECT_OPTIONS.map((opt) => (
                    <button
                      key={opt.value}
                      type="button"
                      onClick={() => setNewAspect(opt.value)}
                      className={`rounded-full px-2 py-0.5 text-[10px] font-medium tabular-nums transition-colors motion-reduce:transition-none ${
                        newAspect === opt.value
                          ? "bg-laranja-criacao/25 text-areia ring-1 ring-laranja-criacao/50"
                          : "bg-carbono text-areia/60 hover:text-areia"
                      }`}
                    >
                      {opt.label}
                    </button>
                  ))}
                </div>
                <button
                  type="submit"
                  disabled={busyId === "create"}
                  className="rounded-full bg-primary px-3 py-1 text-[11px] font-medium text-areia hover:brightness-110 disabled:opacity-45"
                >
                  {busyId === "create" ? "Creating..." : "Create"}
                </button>
              </div>
            </form>
          )}

          {errorMsg && (
            <div className="mt-2 rounded-lg border border-vermelho-foco/45 bg-vermelho-foco/15 px-2.5 py-1.5 text-[11px] text-areia">
              {errorMsg}
            </div>
          )}

          {/* Project List Header & Search */}
          <div className="mt-3 border-t border-areia/10 pt-2.5">
            <div className="mb-2 flex items-center justify-between text-[10px] font-semibold uppercase tracking-[0.14em] text-areia/50">
              <span>All Projects</span>
              <span className="tabular-nums text-areia/45">{projects.length}</span>
            </div>

            {projects.length > 4 && (
              <div className="relative mb-2">
                <Search className="pointer-events-none absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-areia/40" />
                <input
                  type="text"
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                  placeholder="Filter projects..."
                  className="w-full rounded-xl border border-areia/10 bg-void/75 py-1.5 pr-3 pl-8 text-xs text-areia placeholder:text-areia/35 outline-none focus:border-laranja-criacao/60"
                />
              </div>
            )}

            <div className="max-h-64 space-y-1 overflow-y-auto pr-0.5">
              {sortedProjects.length === 0 ? (
                <div className="rounded-xl border border-dashed border-areia/10 py-5 text-center text-[11px] text-areia/45">
                  No matching projects.
                </div>
              ) : (
                sortedProjects.map((proj) => {
                  const isCurrent = proj.id === currentProject.id;
                  const isBusy = busyId === proj.id;
                  return (
                    <button
                      key={proj.id}
                      type="button"
                      disabled={busyId !== null}
                      onClick={() => void handleSwitchProject(proj.id)}
                      className={`flex w-full items-center justify-between gap-2.5 rounded-xl border px-2.5 py-2 text-left transition-colors motion-reduce:transition-none ${
                        isCurrent
                          ? "border-laranja-criacao/45 bg-accent text-areia"
                          : "border-transparent bg-void/45 text-areia/85 hover:border-areia/12 hover:bg-accent"
                      }`}
                    >
                      <div className="flex min-w-0 items-center gap-2">
                        <span
                          className={`h-1.5 w-1.5 shrink-0 rounded-full ${
                            isCurrent ? "bg-laranja-criacao" : "bg-areia/20"
                          }`}
                        />
                        <div className="min-w-0">
                          <div className="truncate text-xs font-medium">{proj.name}</div>
                          <div className="mt-0.5 flex items-center gap-1.5 text-[10px] tabular-nums text-areia/50">
                            <span className="text-amarelo-ideias/80">
                              {ASPECT_LABELS[proj.aspect_preset]}
                            </span>
                            <span>·</span>
                            <span>{formatDuration(proj.duration_sec)}</span>
                            <span>·</span>
                            <span>rev {proj.revision}</span>
                            {proj.updated_at && (
                              <>
                                <span>·</span>
                                <span>{formatRelativeTime(proj.updated_at)}</span>
                              </>
                            )}
                          </div>
                        </div>
                      </div>
                      {isBusy ? (
                        <span className="shrink-0 text-[10px] text-amarelo-ideias">...</span>
                      ) : isCurrent ? (
                        <Check className="h-3.5 w-3.5 shrink-0 text-amarelo-ideias" />
                      ) : null}
                    </button>
                  );
                })
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
