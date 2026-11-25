import { useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowUp,
  Clock,
  Cpu,
  Film,
  FileText,
  Image as ImageIcon,
  KeyRound,
  Loader2,
  Music,
  Palette,
  Paperclip,
  Sparkles,
  X,
} from "lucide-react";
import type {
  AgentStatusResponse,
  AspectPreset,
  BrandPlaybook,
  HealthResponse,
  ModelAdvisorResponse,
  Project,
  ProjectSummary,
  ProjectTemplateInfo,
} from "@solaris/engine-client";
import { isOmniModalModel } from "../AiSettingsModal";
import { SolarisWordmark } from "../SolarisBrand";
import { SolarSelect } from "../ui/SolarSelect";

interface PreEditorChatProps {
  health: HealthResponse | null;
  agentStatus: AgentStatusResponse | null;
  modelAdvisor: ModelAdvisorResponse | null;
  selectedModelOverride: string;
  onSelectModelOverride: (modelId: string) => void;
  brands: BrandPlaybook[];
  templates: ProjectTemplateInfo[];
  recentProjects: ProjectSummary[];
  draftProject: Project | null;
  draftUploading: boolean;
  selectedBrandId: string;
  onSelectBrand: (id: string) => void;
  onOpenBrandDrawer: () => void;
  onOpenAiSettings: () => void;
  onFilesDropped: (files: File[], aspect: AspectPreset) => Promise<void>;
  onRemoveFile?: (file: File) => Promise<void>;
  onOpenExistingProject: (projectId: string) => Promise<void>;
  onLaunchStudio: (params: {
    prompt: string;
    aspect: AspectPreset;
    files: File[];
    agentMode: "openrouter" | "mcp";
    templateId?: string;
    templateName?: string;
  }) => Promise<void>;
}

const ASPECT_OPTIONS: Array<{ value: AspectPreset; label: string }> = [
  { value: "vertical9x16", label: "9:16 Short" },
  { value: "landscape16x9", label: "16:9 Wide" },
  { value: "square1x1", label: "1:1 Square" },
];

export function PreEditorChat({
  health,
  agentStatus,
  modelAdvisor,
  selectedModelOverride,
  onSelectModelOverride,
  brands,
  templates,
  recentProjects,
  draftProject,
  draftUploading,
  selectedBrandId,
  onSelectBrand,
  onOpenBrandDrawer,
  onOpenAiSettings,
  onFilesDropped,
  onRemoveFile,
  onOpenExistingProject,
  onLaunchStudio,
}: PreEditorChatProps) {
  const [prompt, setPrompt] = useState("");
  const [aspect, setAspect] = useState<AspectPreset>("vertical9x16");
  const [selectedTemplateId, setSelectedTemplateId] = useState<string | null>(null);
  const [files, setFiles] = useState<File[]>([]);
  const [agentMode, setAgentMode] = useState<"openrouter" | "mcp">("openrouter");
  const [submitting, setSubmitting] = useState(false);
  const [recentOpen, setRecentOpen] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const recentMenuRef = useRef<HTMLDivElement | null>(null);

  const indexedMedia = draftProject?.media_pool ?? [];
  const hasAttachments = files.length > 0;

  const indexingStats = useMemo(() => {
    let analyzed = 0;
    let words = 0;
    let silences = 0;
    for (const m of indexedMedia) {
      if (m.intelligence.analyzed) analyzed += 1;
      words += m.intelligence.transcript.length;
      silences += m.intelligence.silences.length;
    }
    return { total: indexedMedia.length, analyzed, words, silences };
  }, [indexedMedia]);

  const activeDirectorCandidate = useMemo(() => {
    const candidates = modelAdvisor?.director.candidates ?? [];
    const activeId = selectedModelOverride || modelAdvisor?.director.primary || "";
    return candidates.find((c) => c.id === activeId) ?? candidates[0] ?? null;
  }, [modelAdvisor, selectedModelOverride]);

  const activeIsOmni = activeDirectorCandidate
    ? isOmniModalModel(activeDirectorCandidate)
    : /gemini|muse-ember|mimo-v2\.6/i.test(
        selectedModelOverride || modelAdvisor?.director.primary || "",
      );

  useEffect(() => {
    if (!recentOpen) return;
    const onPointerDown = (e: PointerEvent) => {
      if (
        recentMenuRef.current &&
        !recentMenuRef.current.contains(e.target as Node)
      ) {
        setRecentOpen(false);
      }
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setRecentOpen(false);
    };
    window.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [recentOpen]);

  const handleFilesAdded = (incoming: FileList | null) => {
    if (!incoming || incoming.length === 0) return;
    const added = Array.from(incoming);
    setFiles((prev) => [...prev, ...added]);
    setErrorMsg(null);
    void onFilesDropped(added, aspect).catch((err) => {
      setErrorMsg(
        err instanceof Error ? err.message : "Failed to upload media for background indexing",
      );
    });
  };

  const handleRemoveFile = (index: number) => {
    const removed = files[index];
    setFiles((prev) => prev.filter((_, idx) => idx !== index));
    if (removed && onRemoveFile) {
      void onRemoveFile(removed).catch((err) => {
        setErrorMsg(
          err instanceof Error ? err.message : "Failed to remove media from draft",
        );
      });
    }
  };

  const canSubmit =
    !submitting && (Boolean(prompt.trim()) || hasAttachments || Boolean(selectedTemplateId));

  const handleSubmit = async () => {
    if (!canSubmit) return;
    setSubmitting(true);
    setErrorMsg(null);
    const tpl = selectedTemplateId
      ? templates.find((t) => t.id === selectedTemplateId)
      : undefined;
    try {
      await onLaunchStudio({
        prompt: prompt.trim(),
        aspect,
        files,
        agentMode,
        templateId: selectedTemplateId ?? undefined,
        templateName: tpl?.name,
      });
    } catch (err) {
      setErrorMsg(
        err instanceof Error ? err.message : "Failed to connect to Solaris Engine",
      );
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="relative flex min-h-dvh flex-col overflow-y-auto bg-void text-areia">
      {/* Ambient bottom-right solar ember bloom that recedes as media attaches */}
      <div
        aria-hidden="true"
        className="pointer-events-none fixed inset-0 transition-opacity duration-500 motion-reduce:transition-none"
        style={{
          opacity: hasAttachments ? 0.25 : 0.85,
          background:
            "radial-gradient(62% 52% at 86% 92%, rgba(240,90,0,0.20) 0%, rgba(195,24,24,0.11) 42%, transparent 74%)",
        }}
      />

      {/* Compact 64px translucent header */}
      <header className="sticky top-0 z-20 flex h-16 shrink-0 items-center justify-between gap-3 border-b border-areia/[0.07] bg-void/70 px-5 backdrop-blur-md sm:px-6">
        <SolarisWordmark />

        <div className="flex items-center gap-2">
          <div
            title={
              health?.toolchain.ffmpeg_available
                ? `Solaris Core v${health.version} · FFmpeg ready (${health.toolchain.ffmpeg_version ?? "detected"})`
                : health
                  ? `Solaris Core v${health.version} · FFmpeg not found on PATH or SOLARIS_FFMPEG_PATH`
                  : "Connecting to local Solaris engine"
            }
            className="hidden h-8 items-center gap-2 whitespace-nowrap rounded-full border border-areia/10 bg-carbono/65 px-3 text-[11px] text-areia/75 lg:flex"
          >
            <span
              className={`h-1.5 w-1.5 rounded-full ${
                health?.toolchain.ffmpeg_available
                  ? "bg-amarelo-ideias solar-pulse-dot"
                  : health
                    ? "bg-laranja-criacao"
                    : "bg-vermelho-foco"
              }`}
            />
            <span>
              {health
                ? health.toolchain.ffmpeg_available
                  ? "Engine Ready"
                  : "FFmpeg Standby"
                : "Connecting"}
            </span>
          </div>

          <button
            type="button"
            onClick={onOpenAiSettings}
            className="flex h-8 items-center gap-1.5 whitespace-nowrap rounded-full border border-areia/12 bg-carbono/70 px-3 text-xs text-areia/85 transition-colors hover:border-amarelo-ideias/45 hover:bg-accent motion-reduce:transition-none"
            title="Configure OpenRouter API Key, Model Advisor & External MCP Agent"
          >
            <KeyRound className="h-3.5 w-3.5 text-amarelo-ideias" />
            <span className="hidden sm:inline">
              {agentStatus?.openrouter_configured ? "AI Connected" : "AI & Keys"}
            </span>
          </button>

          {recentProjects.length > 0 && (
            <div ref={recentMenuRef} className="relative">
              <button
                type="button"
                onClick={() => setRecentOpen((v) => !v)}
                className="flex h-8 items-center gap-1.5 whitespace-nowrap rounded-full border border-areia/12 bg-carbono/70 px-3 text-xs text-areia/85 transition-colors hover:border-laranja-criacao/45 hover:bg-accent motion-reduce:transition-none"
              >
                <Clock className="h-3.5 w-3.5 text-amarelo-ideias" />
                <span>Recent ({recentProjects.length})</span>
              </button>
              {recentOpen && (
                <div className="absolute right-0 mt-2 w-72 rounded-2xl border border-areia/14 bg-carbono/95 p-2 shadow-2xl backdrop-blur-xl">
                  <div className="px-2.5 py-1.5 text-[10px] font-semibold uppercase tracking-[0.14em] text-areia/55">
                    Saved Projects
                  </div>
                  <div className="max-h-64 space-y-1 overflow-y-auto">
                    {recentProjects.map((proj) => (
                      <button
                        key={proj.id}
                        type="button"
                        onClick={() => {
                          setRecentOpen(false);
                          setErrorMsg(null);
                          void onOpenExistingProject(proj.id).catch((err) => {
                            setErrorMsg(
                              err instanceof Error
                                ? err.message
                                : "Failed to open project",
                            );
                          });
                        }}
                        className="flex w-full items-center justify-between gap-2 rounded-xl px-2.5 py-2 text-left text-xs text-areia hover:bg-accent"
                      >
                        <span className="truncate font-medium">{proj.name}</span>
                        <span className="shrink-0 tabular-nums text-[10px] text-areia/50">
                          rev {proj.revision}
                        </span>
                      </button>
                    ))}
                  </div>
                </div>
              )}
            </div>
          )}

          <button
            type="button"
            onClick={onOpenBrandDrawer}
            className="flex h-8 items-center gap-1.5 whitespace-nowrap rounded-full border border-areia/12 bg-carbono/70 px-3 text-xs text-areia/85 transition-colors hover:border-laranja-criacao/45 hover:bg-accent motion-reduce:transition-none"
          >
            <Palette className="h-3.5 w-3.5 text-laranja-criacao" />
            <span className="hidden md:inline">Brand Playbook</span>
          </button>
        </div>
      </header>

      {/* Unified Centered Intake Stage (Hero + Composer + Starter Chips) */}
      <main className="relative z-10 mx-auto flex w-full max-w-3xl flex-1 flex-col items-center justify-center px-4 pb-16 pt-8 sm:pb-20 sm:pt-10">
        {/* Hero Headline (clean editorial stack without duplicate center orb) */}
        <div className="flex max-w-2xl flex-col items-center text-center">
          <h1 className="text-[clamp(1.95rem,3.6vw,2.85rem)] font-light leading-[1.12] tracking-[-0.03em] text-areia">
            Direct your next cut with{" "}
            <span className="font-normal text-solar-gradient">Solaris</span>
          </h1>
          <p className="mt-2.5 max-w-lg text-sm leading-relaxed text-areia/60">
            Drop A-roll, B-roll, voiceovers, music, or scripts. Attach a brand playbook
            and let Solaris or your MCP agent cut the timeline.
          </p>
        </div>

        {/* Floating 24px Carbono Glass MediaComposer */}
        <section className="mt-8 w-full">
          {errorMsg && (
            <div className="mb-3 rounded-xl border border-vermelho-foco/40 bg-vermelho-foco/15 px-3.5 py-2 text-xs text-areia">
              {errorMsg}
            </div>
          )}

          {/* Background Media Indexing Live Progress Pill */}
          {(draftUploading || (files.length > 0 && indexingStats.total > 0)) && (
            <div className="mb-2.5 flex flex-wrap items-center gap-2 rounded-2xl border border-amarelo-ideias/25 bg-carbono/80 px-3.5 py-2 text-[11px] text-areia/85 backdrop-blur-md">
              {draftUploading || indexingStats.analyzed < indexingStats.total ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin text-amarelo-ideias motion-reduce:animate-none" />
              ) : (
                <Sparkles className="h-3.5 w-3.5 text-amarelo-ideias" />
              )}
              <span className="tabular-nums">
                Indexed {indexingStats.analyzed}/{Math.max(indexingStats.total, files.length)} clips ·
                Transcribed {indexingStats.words} words · Found {indexingStats.silences} silences
              </span>
            </div>
          )}


          <div
            onDragOver={(e) => e.preventDefault()}
            onDrop={(e) => {
              e.preventDefault();
              handleFilesAdded(e.dataTransfer.files);
            }}
            className="rounded-[24px] border border-areia/14 bg-carbono/82 shadow-[0_24px_64px_-24px_rgba(0,0,0,0.85)] backdrop-blur-xl"
          >
            {/* Integrated Top Mode & Model Context Strip */}
            <div className="flex flex-wrap items-center justify-between gap-2 border-b border-areia/[0.08] px-4 py-2">
              <div className="flex min-w-0 items-center gap-2">
                {agentMode === "openrouter" ? (
                  <>
                    <span
                      title={
                        activeIsOmni
                          ? "Sees raw footage, hears pacing, and inspects QA frames natively"
                          : "Fast, low-cost programmatic video & GFX editing using keyframes, transcripts, and silence/beat telemetry"
                      }
                      className={`inline-flex shrink-0 items-center gap-1.5 rounded-full px-2.5 py-0.5 text-[10px] font-semibold uppercase tracking-[0.1em] ${
                        activeIsOmni
                          ? "bg-amarelo-ideias/15 text-amarelo-ideias ring-1 ring-amarelo-ideias/30"
                          : "bg-laranja-criacao/15 text-areia/90 ring-1 ring-laranja-criacao/35"
                      }`}
                    >
                      <span
                        className={`h-1.5 w-1.5 rounded-full ${
                          activeIsOmni ? "bg-amarelo-ideias" : "bg-laranja-criacao"
                        }`}
                      />
                      {activeIsOmni ? "Omni-Modal" : "Programmatic"}
                    </span>
                    {modelAdvisor ? (
                      <SolarSelect
                        variant="ghost"
                        aria-label="OpenRouter Swarm Model"
                        value={selectedModelOverride}
                        onChange={(e) => onSelectModelOverride(e.target.value)}
                        className="max-w-[235px] sm:max-w-[270px]"
                      >
                        <option value="" className="bg-carbono">
                          Auto · {modelAdvisor.director.primary}
                        </option>
                        {modelAdvisor.director.candidates.slice(0, 12).map((c) => {
                          const price =
                            typeof c.prompt_usd_per_m === "number"
                              ? ` · $${c.prompt_usd_per_m.toFixed(2)}/M`
                              : "";
                          return (
                            <option key={c.id} value={c.id} className="bg-carbono">
                              {c.id} ({isOmniModalModel(c) ? "Omni" : "Prog"}){price}
                            </option>
                          );
                        })}
                      </SolarSelect>
                    ) : (
                      <span className="truncate text-[11px] text-areia/60">
                        OpenRouter Swarm
                      </span>
                    )}
                  </>
                ) : (
                  <div className="flex items-center gap-2 text-[11px] text-areia/80">
                    <span className="inline-flex shrink-0 items-center gap-1.5 rounded-full bg-laranja-criacao/15 px-2.5 py-0.5 text-[10px] font-semibold uppercase tracking-[0.1em] text-areia ring-1 ring-laranja-criacao/35">
                      <span className="h-1.5 w-1.5 rounded-full bg-laranja-criacao" />
                      External MCP
                    </span>
                    <span className="hidden truncate text-areia/60 sm:inline">
                      Drive the timeline from Codex, Antigravity, or Claude Code
                    </span>
                  </div>
                )}
              </div>

              <div className="inline-flex shrink-0 items-center rounded-full border border-areia/10 bg-void/70 p-0.5 text-[10px]">
                <button
                  type="button"
                  onClick={() => setAgentMode("openrouter")}
                  className={`flex items-center gap-1 rounded-full px-2.5 py-1 font-medium transition-colors motion-reduce:transition-none ${
                    agentMode === "openrouter"
                      ? "bg-grafite text-areia shadow-xs"
                      : "text-areia/55 hover:text-areia/85"
                  }`}
                >
                  <Cpu className="h-3 w-3 text-amarelo-ideias" />
                  <span>Swarm</span>
                </button>
                <button
                  type="button"
                  onClick={() => setAgentMode("mcp")}
                  className={`rounded-full px-2.5 py-1 font-medium transition-colors motion-reduce:transition-none ${
                    agentMode === "mcp"
                      ? "bg-grafite text-areia shadow-xs"
                      : "text-areia/55 hover:text-areia/85"
                  }`}
                >
                  MCP
                </button>
              </div>
            </div>

            <div className="p-3.5">
              {/* Attachment Rail */}
              {files.length > 0 && (
                <div className="mb-3 flex gap-2 overflow-x-auto pb-1">
                  {files.map((file, idx) => (
                    <div
                      key={`${file.name}-${idx}`}
                      className="flex shrink-0 items-center gap-2 rounded-xl border border-areia/12 bg-void/70 px-2.5 py-1.5 text-xs"
                    >
                      <FileKindIcon name={file.name} />
                      <span className="max-w-[140px] truncate text-areia/90">
                        {file.name}
                      </span>
                      <span className="tabular-nums text-[10px] text-areia/45">
                        {(file.size / (1024 * 1024)).toFixed(1)} MB
                      </span>
                      <button
                        type="button"
                        onClick={() => handleRemoveFile(idx)}
                        className="rounded-full p-0.5 text-areia/50 hover:bg-grafite hover:text-areia"
                        aria-label={`Remove ${file.name}`}
                      >
                        <X className="h-3 w-3" />
                      </button>
                    </div>
                  ))}
                </div>
              )}

              <textarea
                rows={3}
                value={prompt}
                onChange={(e) => setPrompt(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                    void handleSubmit();
                  }
                }}
                placeholder="Describe the edit: e.g. 'Turn these 12 S-Log3 clips into a 20s 9:16 hook reel with punch-ins, Neon Glow captions, -14 LUFS mix, and animated stat cards...'"
                className="w-full resize-none bg-transparent px-1.5 py-1 text-sm leading-relaxed text-areia placeholder:text-areia/40 outline-none"
              />

              {/* Single-Row Bottom Action Bar (Send always anchored right) */}
              <div className="mt-2.5 flex flex-wrap items-center justify-between gap-2 border-t border-areia/10 pt-2.5">
                <div className="flex flex-wrap items-center gap-1.5">
                  <input
                    ref={fileInputRef}
                    type="file"
                    multiple
                    accept="video/*,image/*,audio/*,.pdf,.md,.txt"
                    onChange={(e) => {
                      handleFilesAdded(e.target.files);
                      e.currentTarget.value = "";
                    }}
                    className="hidden"
                  />
                  <button
                    type="button"
                    onClick={() => fileInputRef.current?.click()}
                    className="flex h-9 items-center gap-1.5 whitespace-nowrap rounded-full border border-areia/12 bg-grafite/75 px-3 text-xs text-areia/90 transition-colors hover:border-laranja-criacao/45 hover:bg-accent motion-reduce:transition-none"
                  >
                    <Paperclip className="h-3.5 w-3.5 text-laranja-criacao" />
                    <span>Attach Media</span>
                  </button>

                  <SolarSelect
                    value={aspect}
                    onChange={(e) => setAspect(e.target.value as AspectPreset)}
                    aria-label="Aspect Ratio"
                  >
                    {ASPECT_OPTIONS.map((opt) => (
                      <option key={opt.value} value={opt.value} className="bg-carbono">
                        {opt.label}
                      </option>
                    ))}
                  </SolarSelect>

                  <SolarSelect
                    value={selectedBrandId}
                    onChange={(e) => onSelectBrand(e.target.value)}
                    aria-label="Brand Playbook"
                    className="max-w-[175px]"
                  >
                    {brands.map((b) => (
                      <option key={b.id} value={b.id} className="bg-carbono">
                        {b.name}
                      </option>
                    ))}
                  </SolarSelect>
                </div>

                <div className="ml-auto flex items-center gap-2">
                  <span className="hidden text-[10px] text-areia/40 sm:inline">
                    ⌘↵ to launch
                  </span>
                  <button
                    type="button"
                    disabled={!canSubmit}
                    onClick={() => void handleSubmit()}
                    className="grid h-9 w-9 shrink-0 place-items-center rounded-full bg-primary text-areia shadow-[0_0_20px_-4px_rgba(240,90,0,0.75)] transition-transform duration-150 hover:scale-105 disabled:opacity-35 disabled:hover:scale-100 motion-reduce:transition-none"
                    aria-label="Launch Studio"
                  >
                    {submitting ? (
                      <Loader2 className="h-4 w-4 animate-spin motion-reduce:animate-none" />
                    ) : (
                      <ArrowUp className="h-4 w-4" />
                    )}
                  </button>
                </div>
              </div>
            </div>
          </div>
        </section>

        {/* Starter Templates neatly tucked below the Composer */}
        {templates.length > 0 && (
          <div className="mt-4 flex flex-wrap items-center justify-center gap-2">
            <span className="text-[10px] font-medium uppercase tracking-[0.14em] text-areia/40">
              Templates
            </span>
            {templates.map((tpl) => {
              const active = selectedTemplateId === tpl.id;
              return (
                <button
                  key={tpl.id}
                  type="button"
                  onClick={() => {
                    if (active) {
                      setSelectedTemplateId(null);
                    } else {
                      setSelectedTemplateId(tpl.id);
                      setAspect(tpl.aspect_preset);
                      if (!prompt.trim()) {
                        setPrompt(tpl.recommended_prompt);
                      }
                    }
                  }}
                  title={tpl.description}
                  className={`flex h-8 items-center gap-2 rounded-full border px-3.5 text-xs transition-colors motion-reduce:transition-none ${
                    active
                      ? "border-laranja-criacao bg-laranja-criacao/20 text-areia"
                      : "border-areia/12 bg-carbono/65 text-areia/80 hover:border-amarelo-ideias/45 hover:bg-accent"
                  }`}
                >
                  <Sparkles className="h-3 w-3 text-amarelo-ideias" />
                  <span>{tpl.name}</span>
                  <span className="tabular-nums text-[10px] text-areia/50">
                    {tpl.aspect_preset === "vertical9x16"
                      ? "9:16"
                      : tpl.aspect_preset === "square1x1"
                        ? "1:1"
                        : "16:9"}
                  </span>
                </button>
              );
            })}
          </div>
        )}
      </main>
    </div>
  );
}

function FileKindIcon({ name }: { name: string }) {
  const ext = name.split(".").pop()?.toLowerCase() ?? "";
  if (["mp4", "mov", "webm", "mkv"].includes(ext)) {
    return <Film className="h-3.5 w-3.5 text-track-v1" />;
  }
  if (["png", "jpg", "jpeg", "webp", "svg"].includes(ext)) {
    return <ImageIcon className="h-3.5 w-3.5 text-track-v2" />;
  }
  if (["wav", "mp3", "m4a", "aac", "ogg"].includes(ext)) {
    return <Music className="h-3.5 w-3.5 text-track-a1" />;
  }
  return <FileText className="h-3.5 w-3.5 text-amarelo-ideias" />;
}
