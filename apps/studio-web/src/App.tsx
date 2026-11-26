import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useRef,
  useState,
  type FormEvent,
} from "react";
import { Lock, KeyRound } from "lucide-react";
import {
  EngineApiError,
  SolarisEngineClient,
  type AgentStatusResponse,
  type AspectPreset,
  type BrandPlaybook,
  type EngineEventEnvelope,
  type ExportJobStatus,
  type HealthResponse,
  type LogEntry,
  type ModelAdvisorResponse,
  type Project,
  type ProjectSummary,
  type ProjectTemplateInfo,
  type QaReport,
  type SavedCodeAsset,
  type SwarmRunStatus,
} from "@solaris/engine-client";
import { AiSettingsModal } from "./components/AiSettingsModal";
import { BrandPlaybookDrawer } from "./components/BrandPlaybookDrawer";
import { PreEditorChat } from "./components/chat/PreEditorChat";
import { SolarisWordmark } from "./components/SolarisBrand";
import { createEngineClient } from "./lib/engineBridge";

const StudioWorkspace = lazy(() => import("./components/studio/StudioWorkspace"));

function isUnauthorizedError(err: unknown): boolean {
  return (
    (err instanceof EngineApiError && err.status === 401) ||
    (err instanceof Error && err.message.includes("Solaris Engine 401"))
  );
}

const EXPORT_STATE_RANK: Record<ExportJobStatus["state"], number> = {
  queued: 0,
  rendering_gfx: 1,
  compositing: 2,
  completed: 3,
  failed: 3,
};

const SWARM_STATE_RANK: Record<SwarmRunStatus["state"], number> = {
  queued: 0,
  planning: 1,
  executing_spine: 2,
  executing_specialists: 3,
  running_qa: 4,
  completed: 5,
  failed: 5,
  cancelled: 5,
};

function mergeLogEntries(existing: LogEntry[], incoming: LogEntry[]): LogEntry[] {
  const byId = new Map<string, LogEntry>();
  for (const entry of existing) {
    byId.set(entry.id, entry);
  }
  for (const entry of incoming) {
    byId.set(entry.id, entry);
  }
  const merged = Array.from(byId.values());
  merged.sort((a, b) => a.timestamp.localeCompare(b.timestamp));
  return merged.length > 400 ? merged.slice(-400) : merged;
}

export function App() {
  const [client, setClient] = useState<SolarisEngineClient | null>(null);
  const [authRequired, setAuthRequired] = useState(false);
  const [authTokenInput, setAuthTokenInput] = useState("");
  const [authError, setAuthError] = useState<string | null>(null);
  const [authSubmitting, setAuthSubmitting] = useState(false);
  const [authSeq, setAuthSeq] = useState(0);
  const [ticketVersion, setTicketVersion] = useState(0);
  const [health, setHealth] = useState<HealthResponse | null>(null);
  const [brands, setBrands] = useState<BrandPlaybook[]>([]);
  const [recentProjects, setRecentProjects] = useState<ProjectSummary[]>([]);
  const [selectedBrandId, setSelectedBrandId] = useState("solaris-default");
  const [assets, setAssets] = useState<SavedCodeAsset[]>([]);
  const [activeProject, setActiveProject] = useState<Project | null>(null);
  const [draftProject, setDraftProject] = useState<Project | null>(null);
  const [draftUploading, setDraftUploading] = useState(false);
  const [events, setEvents] = useState<EngineEventEnvelope[]>([]);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [wsConnected, setWsConnected] = useState(false);
  const [brandDrawerOpen, setBrandDrawerOpen] = useState(false);
  const [aiSettingsOpen, setAiSettingsOpen] = useState(false);
  const [exportJob, setExportJob] = useState<ExportJobStatus | null>(null);
  const [agentStatus, setAgentStatus] = useState<AgentStatusResponse | null>(null);
  const [modelAdvisor, setModelAdvisor] = useState<ModelAdvisorResponse | null>(null);
  const [selectedModelOverride, setSelectedModelOverride] = useState<string>("");
  const [templates, setTemplates] = useState<ProjectTemplateInfo[]>([]);
  const [activeSwarmRun, setActiveSwarmRun] = useState<SwarmRunStatus | null>(null);
  const [swarmStartError, setSwarmStartError] = useState<string | null>(null);
  const [qaReport, setQaReport] = useState<QaReport | null>(null);
  const [externalPlayheadSeek, setExternalPlayheadSeek] = useState<{
    timeSec: number;
    seq: number;
  } | null>(null);

  const exportJobRef = useRef<ExportJobStatus | null>(null);
  exportJobRef.current = exportJob;
  const activeSwarmRunRef = useRef<SwarmRunStatus | null>(null);
  activeSwarmRunRef.current = activeSwarmRun;
  const draftProjectRef = useRef<Project | null>(null);
  draftProjectRef.current = draftProject;
  const draftCreatePromiseRef = useRef<Promise<Project> | null>(null);
  const draftUploadCountRef = useRef(0);
  const draftFileMediaMapRef = useRef<Map<File, string>>(new Map());
  const removedDraftFilesRef = useRef<WeakSet<File>>(new WeakSet());
  const activeProjectIdRef = useRef<string | null>(null);
  activeProjectIdRef.current = activeProject?.id ?? null;
  const wsConnectedRef = useRef(false);
  wsConnectedRef.current = wsConnected;

  const applyExportJobIfNewer = useCallback((next: ExportJobStatus | null) => {
    setExportJob((prev) => {
      if (!next) return null;
      if (!prev || prev.id !== next.id) return next;
      const prevRank = EXPORT_STATE_RANK[prev.state];
      const nextRank = EXPORT_STATE_RANK[next.state];
      if (nextRank < prevRank) return prev;
      if (nextRank === prevRank && next.progress < prev.progress) return prev;
      return next;
    });
  }, []);

  const applySwarmRunIfNewer = useCallback((next: SwarmRunStatus | null) => {
    setActiveSwarmRun((prev) => {
      if (!next) return null;
      if (!prev || prev.run_id !== next.run_id) return next;
      const prevRank = SWARM_STATE_RANK[prev.state];
      const nextRank = SWARM_STATE_RANK[next.state];
      if (nextRank < prevRank) return prev;
      if (nextRank === prevRank && next.ops_committed < prev.ops_committed) return prev;
      return next;
    });
  }, []);

  useEffect(() => {
    let mounted = true;
    let created: SolarisEngineClient | null = null;
    void createEngineClient().then((c) => {
      if (!mounted) {
        c.dispose();
        return;
      }
      created = c;
      setClient(c);
    });
    return () => {
      mounted = false;
      created?.dispose();
    };
  }, []);

  useEffect(() => {
    if (!client) return;
    return client.onTicketChange(() => setTicketVersion((v) => v + 1));
  }, [client]);

  const applyProjectIfNewer = useCallback((next: Project | null) => {
    setActiveProject((prev) => {
      if (!next) return null;
      if (prev && prev.id === next.id && next.revision < prev.revision) {
        return prev;
      }
      return next;
    });
  }, []);

  const refreshModelAdvisor = useCallback(async () => {
    if (!client) return;
    try {
      const adv = await client.getRecommendedModels();
      setModelAdvisor(adv);
    } catch {
      // Keep existing recommendations if offline
    }
  }, [client]);

  useEffect(() => {
    if (!client) return;
    let cancelled = false;
    void client
      .health()
      .then((h) => {
        if (!cancelled) setHealth(h);
      })
      .catch(() => {
        if (!cancelled) setHealth(null);
      });

    const loadAuthed = async () => {
      try {
        await client.establishSession(true).catch((err) => {
          if (isUnauthorizedError(err)) throw err;
        });
        const [brandList, assetList, status, tplList, projectList] =
          await Promise.all([
            client.listBrands(),
            client.listAssets(),
            client.getAgentStatus(),
            client.listTemplates(),
            client.listProjects(),
          ]);
        if (cancelled) return;
        setAuthRequired(false);
        setBrands(brandList);
        if (brandList.length > 0) {
          setSelectedBrandId((prev) =>
            brandList.some((b) => b.id === prev) ? prev : brandList[0].id,
          );
        }
        setAssets(assetList);
        setAgentStatus(status);
        if (status.default_model) {
          setSelectedModelOverride(status.default_model);
        }
        setTemplates(tplList);
        setRecentProjects(projectList);

        void client
          .getRecommendedModels()
          .then((adv) => {
            if (!cancelled) setModelAdvisor(adv);
          })
          .catch(() => {});
      } catch (err) {
        if (cancelled) return;
        if (isUnauthorizedError(err)) {
          setAuthRequired(true);
        }
      }
    };

    void loadAuthed();
    return () => {
      cancelled = true;
    };
  }, [client, authSeq]);

  useEffect(() => {
    if (!client || authRequired) return;
    void client.listProjects().then(setRecentProjects).catch(() => {});
  }, [client, activeProject?.id, authRequired]);

  // Subscribe to background media analysis updates while user is still in PreEditorChat
  useEffect(() => {
    if (!client || activeProject || !draftProject?.id) return;
    const unsubscribe = client.subscribeEvents(draftProject.id, (envelope) => {
      if (
        envelope.payload.event === "op_committed" ||
        envelope.payload.event === "project_reset"
      ) {
        const proj = envelope.payload.project;
        setDraftProject((prev) =>
          prev && prev.id === proj.id && proj.revision < prev.revision ? prev : proj,
        );
      } else if (envelope.payload.event === "media_analyzed") {
        const { media_id, intelligence } = envelope.payload;
        setDraftProject((prev) =>
          prev
            ? {
                ...prev,
                media_pool: prev.media_pool.map((m) =>
                  m.id === media_id ? { ...m, intelligence } : m,
                ),
              }
            : prev,
        );
      }
    });
    return unsubscribe;
  }, [client, activeProject, draftProject?.id]);

  const handleEngineSignIn = async (e: FormEvent) => {
    e.preventDefault();
    if (!client || !authTokenInput.trim() || authSubmitting) return;
    setAuthSubmitting(true);
    setAuthError(null);
    try {
      await client.signIn(authTokenInput.trim());
      setAuthTokenInput("");
      setAuthRequired(false);
      setAuthSeq((s) => s + 1);
    } catch (err) {
      setAuthError(
        isUnauthorizedError(err)
          ? "Invalid server token. Check SOLARIS_AUTH_TOKEN and try again."
          : err instanceof Error
            ? err.message
            : "Failed to authenticate with Solaris Engine.",
      );
    } finally {
      setAuthSubmitting(false);
    }
  };

  useEffect(() => {
    if (!client || !activeProject?.id) return;
    const pid = activeProject.id;
    let active = true;
    void client
      .getLogs({ projectId: pid, limit: 250 })
      .then((initialLogs) => {
        if (active) setLogs((prev) => mergeLogEntries(prev, initialLogs));
      })
      .catch(() => {});

    const unsubscribe = client.subscribeEvents(
      pid,
      (envelope) => {
        if (envelope.payload.event !== "log_emitted") {
          setEvents((prev) => [...prev.slice(-120), envelope]);
        }
        if (
          envelope.payload.event === "op_committed" ||
          envelope.payload.event === "project_reset"
        ) {
          applyProjectIfNewer(envelope.payload.project);
        } else if (envelope.payload.event === "media_analyzed") {
          const { media_id, intelligence } = envelope.payload;
          setActiveProject((prev) =>
            prev
              ? {
                  ...prev,
                  media_pool: prev.media_pool.map((m) =>
                    m.id === media_id ? { ...m, intelligence } : m,
                  ),
                }
              : prev,
          );
        } else if (envelope.payload.event === "export_progress") {
          applyExportJobIfNewer(envelope.payload.job);
        } else if (envelope.payload.event === "swarm_progress") {
          applySwarmRunIfNewer(envelope.payload.run);
          if (envelope.payload.run.qa_report) {
            setQaReport(envelope.payload.run.qa_report);
          }
        } else if (envelope.payload.event === "playhead_seek") {
          const seekTime = envelope.payload.time_sec;
          setExternalPlayheadSeek((prev) => ({
            timeSec: seekTime,
            seq: (prev?.seq ?? 0) + 1,
          }));
        } else if (envelope.payload.event === "log_emitted") {
          const entry = envelope.payload.entry;
          setLogs((prev) => {
            if (prev.some((existing) => existing.id === entry.id)) return prev;
            return [...prev.slice(-399), entry];
          });
        }
      },
      setWsConnected,
      () => {
        void client.getProject(pid).then(applyProjectIfNewer).catch(() => {});
        void client
          .getLogs({ projectId: pid, limit: 250 })
          .then((refreshedLogs) => {
            if (active) setLogs((prev) => mergeLogEntries(prev, refreshedLogs));
          })
          .catch(() => {});
        const curExport = exportJobRef.current;
        if (
          curExport &&
          curExport.state !== "completed" &&
          curExport.state !== "failed"
        ) {
          void client
            .getExportJob(curExport.id)
            .then(applyExportJobIfNewer)
            .catch(() => {});
        }
        const curSwarm = activeSwarmRunRef.current;
        if (
          curSwarm &&
          curSwarm.project_id === pid &&
          curSwarm.state !== "completed" &&
          curSwarm.state !== "failed" &&
          curSwarm.state !== "cancelled"
        ) {
          void client
            .getSwarmRun(pid, curSwarm.run_id)
            .then(applySwarmRunIfNewer)
            .catch(() => {});
        }
      },
    );
    return () => {
      active = false;
      unsubscribe();
    };
  }, [client, activeProject?.id, applyProjectIfNewer, applyExportJobIfNewer, applySwarmRunIfNewer]);

  if (!client) {
    return (
      <div className="grid h-dvh place-items-center bg-void text-xs text-areia/65">
        Initializing Solaris Engine...
      </div>
    );
  }

  const activeBrand =
    brands.find((b) => b.id === selectedBrandId) ?? brands[0] ?? null;

  const activeModelLabel =
    selectedModelOverride ||
    agentStatus?.default_model ||
    modelAdvisor?.director.primary ||
    undefined;

  const handleSaveBrand = async (updated: BrandPlaybook) => {
    const saved = await client.saveBrand(updated);
    setBrands((prev) => prev.map((b) => (b.id === saved.id ? saved : b)));
  };

  const handleCreateBrand = async (created: BrandPlaybook) => {
    const saved = await client.createBrand(created);
    setBrands((prev) => [...prev, saved]);
  };

  const handleBrandRefreshed = (updated: BrandPlaybook) => {
    setBrands((prev) => prev.map((b) => (b.id === updated.id ? updated : b)));
  };

  const handleOpenExistingProject = async (projectId: string) => {
    const proj = await client.getProject(projectId);
    setEvents([]);
    setLogs([]);
    setExportJob(null);
    setActiveSwarmRun(null);
    setSwarmStartError(null);
    setQaReport(null);
    setExternalPlayheadSeek(null);
    setDraftProject(null);
    draftProjectRef.current = null;
    draftCreatePromiseRef.current = null;
    draftFileMediaMapRef.current.clear();
    if (proj.brand_playbook_id) {
      setSelectedBrandId(proj.brand_playbook_id);
    }
    setActiveProject(proj);
  };

  const handleCreateBlankProject = async (name: string, aspect: AspectPreset) => {
    const created = await client.createProject(name.trim(), aspect, selectedBrandId);
    setEvents([]);
    setLogs([]);
    setExportJob(null);
    setActiveSwarmRun(null);
    setSwarmStartError(null);
    setQaReport(null);
    setExternalPlayheadSeek(null);
    setDraftProject(null);
    draftProjectRef.current = null;
    draftCreatePromiseRef.current = null;
    draftFileMediaMapRef.current.clear();
    activeProjectIdRef.current = created.id;
    setActiveProject(created);
    void client.listProjects().then(setRecentProjects).catch(() => {});
  };

  const handleRenameActiveProject = async (name: string) => {
    if (!activeProject) return;
    const updated = await client.updateProjectMetadata(activeProject.id, {
      name: name.trim(),
    });
    applyProjectIfNewer(updated);
    void client.listProjects().then(setRecentProjects).catch(() => {});
  };

  const handleDeleteProject = async (projectId: string) => {
    await client.deleteProject(projectId);
    try {
      window.sessionStorage.removeItem(`solaris.director.history.${projectId}`);
    } catch {
      // Ignore storage access errors
    }
    if (draftProjectRef.current?.id === projectId) {
      setDraftProject(null);
      draftProjectRef.current = null;
      draftCreatePromiseRef.current = null;
      draftFileMediaMapRef.current.clear();
    }
    if (activeProjectIdRef.current === projectId) {
      activeProjectIdRef.current = null;
      setActiveProject(null);
      setEvents([]);
      setLogs([]);
      setExportJob(null);
      setActiveSwarmRun(null);
      setSwarmStartError(null);
      setQaReport(null);
      setExternalPlayheadSeek(null);
    }
    setRecentProjects((prev) => prev.filter((p) => p.id !== projectId));
    void client.listProjects().then(setRecentProjects).catch(() => {});
  };

  const handleSelectBrand = async (brandId: string) => {
    setSelectedBrandId(brandId);
    if (activeProject && activeProject.brand_playbook_id !== brandId) {
      try {
        const res = await client.applyOp(
          activeProject.id,
          { type: "set_brand_playbook", brand_playbook_id: brandId },
          { agent_id: "user", role: "user" },
        );
        applyProjectIfNewer(res.project);
      } catch {
        // Ignore if project closed concurrently
      }
    }
  };

  // Start uploading & indexing media immediately upon drop in PreEditorChat (serialized single-flight draft creation)
  const handlePreEditorFilesDrop = async (incoming: File[], aspect: AspectPreset) => {
    if (incoming.length === 0) return;
    draftUploadCountRef.current += 1;
    setDraftUploading(true);
    try {
      let draft = draftProjectRef.current;
      if (!draft) {
        if (!draftCreatePromiseRef.current) {
          draftCreatePromiseRef.current = client
            .createProject("solaris_draft", aspect, selectedBrandId)
            .then((created) => {
              draftProjectRef.current = created;
              setDraftProject(created);
              return created;
            })
            .catch((err) => {
              draftCreatePromiseRef.current = null;
              throw err;
            });
        }
        draft = await draftCreatePromiseRef.current;
      }

      const imported = await client.importMediaFiles(draft.id, incoming);
      const toDeleteNow: string[] = [];
      for (let i = 0; i < incoming.length; i++) {
        const file = incoming[i];
        const media = imported[i];
        if (!media) continue;
        if (removedDraftFilesRef.current.has(file)) {
          toDeleteNow.push(media.id);
        } else {
          draftFileMediaMapRef.current.set(file, media.id);
        }
      }

      let refreshed: Project | null = null;
      for (const mediaId of toDeleteNow) {
        try {
          refreshed = await client.deleteMedia(draft.id, mediaId);
        } catch {
          // Ignore if already removed
        }
      }
      if (!refreshed) {
        refreshed = await client.getProject(draft.id);
      }
      if (draftProjectRef.current?.id === draft.id) {
        draftProjectRef.current = refreshed;
        setDraftProject(refreshed);
      }
    } finally {
      draftUploadCountRef.current = Math.max(0, draftUploadCountRef.current - 1);
      setDraftUploading(draftUploadCountRef.current > 0);
    }
  };

  const handleRemovePreEditorFile = async (file: File) => {
    removedDraftFilesRef.current.add(file);
    const draft = draftProjectRef.current;
    if (!draft) return;

    let mediaId = draftFileMediaMapRef.current.get(file);
    if (!mediaId) {
      const matched = draft.media_pool.find((m) => m.name === file.name);
      mediaId = matched?.id;
    }
    if (!mediaId) return;

    draftFileMediaMapRef.current.delete(file);
    const updated = await client.deleteMedia(draft.id, mediaId);
    if (draftProjectRef.current?.id === draft.id) {
      draftProjectRef.current = updated;
      setDraftProject(updated);
    }
  };

  const handleLaunchStudio = async ({
    prompt,
    aspect,
    files,
    agentMode,
    templateId,
    templateName,
  }: {
    prompt: string;
    aspect: AspectPreset;
    files: File[];
    agentMode?: "openrouter" | "mcp";
    templateId?: string;
    templateName?: string;
  }) => {
    const userPrompt = prompt.trim();
    const rawDisplayName =
      templateName ||
      (userPrompt ? userPrompt.slice(0, 42) : "Untitled Solaris Edit");
    const cleanName =
      rawDisplayName
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, "_")
        .replace(/^_+|_+$/g, "")
        .slice(0, 40) || "solaris_edit";

    let project: Project;
    if (!draftProjectRef.current && draftCreatePromiseRef.current) {
      try {
        await draftCreatePromiseRef.current;
      } catch {
        // Proceed with fresh project creation below
      }
    }
    const existingDraft = draftProjectRef.current;
    if (
      !templateId &&
      existingDraft &&
      existingDraft.canvas.aspect_preset === aspect &&
      files.length > 0 &&
      existingDraft.media_pool.length > 0
    ) {
      // Reuse already-indexed background draft project and apply real project name & prompt
      project = await client.updateProjectMetadata(existingDraft.id, {
        name: cleanName,
        prompt: userPrompt || undefined,
      });
      if (project.brand_playbook_id !== selectedBrandId) {
        const res = await client.applyOp(
          project.id,
          { type: "set_brand_playbook", brand_playbook_id: selectedBrandId },
          { agent_id: "user", role: "user" },
        );
        project = res.project;
      }
    } else {
      project = await client.createProject(
        cleanName,
        templateId ? undefined : aspect,
        selectedBrandId,
        userPrompt || undefined,
        templateId,
      );
      if (files.length > 0) {
        await client.importMediaFiles(project.id, files);
        project = await client.getProject(project.id);
      }
    }

    setEvents([]);
    setLogs([]);
    setExportJob(null);
    setActiveSwarmRun(null);
    setSwarmStartError(null);
    setQaReport(null);
    setExternalPlayheadSeek(null);
    setDraftProject(null);
    draftProjectRef.current = null;
    draftCreatePromiseRef.current = null;
    draftFileMediaMapRef.current.clear();
    activeProjectIdRef.current = project.id;
    setActiveProject(project);

    // Auto-start Swarm whenever the user provided a directing prompt (including zero-asset programmatic videos!)
    const shouldAutoRunSwarm =
      agentMode === "openrouter" && Boolean(userPrompt);

    if (shouldAutoRunSwarm) {
      const targetPid = project.id;
      void (async () => {
        // Readiness barrier: wait briefly for WebSocket subscription (up to 1.5s)
        const wsDeadline = Date.now() + 1500;
        while (!wsConnectedRef.current && Date.now() < wsDeadline) {
          if (activeProjectIdRef.current !== targetPid) return;
          await new Promise((r) => setTimeout(r, 120));
        }
        if (activeProjectIdRef.current !== targetPid) return;

        // Wait briefly for Stage-1 media analysis before launching Swarm
        const deadline = Date.now() + 3500;
        let current = project;
        while (
          current.media_pool.some((m) => !m.intelligence.analyzed && m.kind !== "document") &&
          Date.now() < deadline
        ) {
          await new Promise((r) => setTimeout(r, 350));
          if (activeProjectIdRef.current !== targetPid) return;
          try {
            current = await client.getProject(targetPid);
            if (activeProjectIdRef.current !== targetPid) return;
            applyProjectIfNewer(current);
          } catch {
            break;
          }
        }
        if (activeProjectIdRef.current !== targetPid) return;

        try {
          const run = await client.startSwarmRun(targetPid, {
            prompt: userPrompt,
            model_override: selectedModelOverride || undefined,
            allow_local_fallback: !(agentStatus?.openrouter_configured ?? false),
          });
          if (activeProjectIdRef.current !== targetPid) return;
          applySwarmRunIfNewer(run);
        } catch (err) {
          if (activeProjectIdRef.current !== targetPid) return;
          setSwarmStartError(
            err instanceof Error ? err.message : "Failed to start Swarm run",
          );
        }
      })();
    }
  };

  return (
    <>
      {activeProject ? (
        <Suspense
          fallback={
            <div className="grid h-dvh place-items-center bg-void text-xs text-areia/65">
              Loading Solaris Studio...
            </div>
          }
        >
          <StudioWorkspace
            client={client}
            ticketVersion={ticketVersion}
            project={activeProject}
            projects={recentProjects}
            activeBrand={activeBrand}
            assets={assets}
            events={events}
            logs={logs}
            onClearLogs={async () => {
              const pid = activeProject.id;
              await client.clearLogs(pid);
              setLogs((prev) => prev.filter((entry) => entry.project_id !== pid));
            }}
            wsConnected={wsConnected}
            exportJob={exportJob}
            agentStatus={agentStatus}
            activeModelLabel={activeModelLabel}
            selectedModelOverride={selectedModelOverride}
            activeSwarmRun={activeSwarmRun}
            swarmError={swarmStartError}
            latestQaReport={qaReport}
            externalPlayheadSeek={externalPlayheadSeek}
            modalsOpen={brandDrawerOpen || aiSettingsOpen || authRequired}
            onSwarmRunChange={applySwarmRunIfNewer}
            onClearSwarmError={() => setSwarmStartError(null)}
            onQaReportChange={setQaReport}
            onExportJobChange={applyExportJobIfNewer}
            onAssetCreated={(asset) =>
              setAssets((prev) => {
                const idx = prev.findIndex((a) => a.id === asset.id);
                if (idx === -1) return [...prev, asset];
                const next = [...prev];
                next[idx] = asset;
                return next;
              })
            }
            onAssetsReplaced={setAssets}
            onProjectUpdate={applyProjectIfNewer}
            onSelectProject={handleOpenExistingProject}
            onCreateBlankProject={handleCreateBlankProject}
            onRenameProject={handleRenameActiveProject}
            onDeleteProject={handleDeleteProject}
            onBackToChat={() => {
              setActiveProject(null);
              setSwarmStartError(null);
            }}
            onOpenBrandDrawer={() => setBrandDrawerOpen(true)}
            onOpenAiSettings={() => setAiSettingsOpen(true)}
          />
        </Suspense>
      ) : (
        <PreEditorChat
          health={health}
          agentStatus={agentStatus}
          modelAdvisor={modelAdvisor}
          selectedModelOverride={selectedModelOverride}
          onSelectModelOverride={(m) => {
            setSelectedModelOverride(m);
            void client
              .setAgentCredentials(
                m ? { default_model: m } : { clear_default_model: true },
              )
              .then(setAgentStatus)
              .catch(() => {});
          }}
          brands={brands}
          templates={templates}
          recentProjects={recentProjects}
          draftProject={draftProject}
          draftUploading={draftUploading}
          selectedBrandId={selectedBrandId}
          onSelectBrand={setSelectedBrandId}
          onOpenBrandDrawer={() => setBrandDrawerOpen(true)}
          onOpenAiSettings={() => setAiSettingsOpen(true)}
          onFilesDropped={handlePreEditorFilesDrop}
          onRemoveFile={handleRemovePreEditorFile}
          onOpenExistingProject={handleOpenExistingProject}
          onDeleteProject={handleDeleteProject}
          onLaunchStudio={handleLaunchStudio}
        />
      )}

      <BrandPlaybookDrawer
        open={brandDrawerOpen}
        client={client}
        brands={brands}
        selectedBrandId={selectedBrandId}
        onSelectBrand={(id) => void handleSelectBrand(id)}
        onClose={() => setBrandDrawerOpen(false)}
        onSave={handleSaveBrand}
        onCreateBrand={handleCreateBrand}
        onBrandRefreshed={handleBrandRefreshed}
      />

      <AiSettingsModal
        open={aiSettingsOpen}
        client={client}
        agentStatus={agentStatus}
        modelAdvisor={modelAdvisor}
        selectedModelOverride={selectedModelOverride}
        onSelectModelOverride={setSelectedModelOverride}
        onAgentStatusChange={setAgentStatus}
        onRefreshModels={refreshModelAdvisor}
        onClose={() => setAiSettingsOpen(false)}
      />

      {authRequired && (
        <div
          role="dialog"
          aria-modal="true"
          aria-labelledby="solaris-auth-title"
          className="fixed inset-0 z-50 grid place-items-center bg-void/85 p-4 backdrop-blur-xl"
        >
          <form
            onSubmit={(e) => void handleEngineSignIn(e)}
            className="w-full max-w-md rounded-2xl border border-areia/15 bg-carbono/95 p-6 shadow-2xl"
          >
            <div className="mb-4 flex items-center justify-between">
              <SolarisWordmark />
              <span className="inline-flex items-center gap-1.5 rounded-full border border-laranja-criacao/35 bg-laranja-criacao/10 px-2.5 py-0.5 text-[10px] font-semibold uppercase tracking-widest text-amarelo-ideias">
                <Lock className="h-3 w-3" />
                Protected Engine
              </span>
            </div>
            <h2
              id="solaris-auth-title"
              className="text-base font-medium tracking-tight text-areia"
            >
              Sign in to Solaris Engine
            </h2>
            <p className="mt-1.5 text-xs leading-relaxed text-areia/65">
              This server requires an access token (<span className="text-areia/85">SOLARIS_AUTH_TOKEN</span>).
              Your token is exchanged for an HttpOnly session cookie and is never stored in localStorage.
            </p>
            <div className="mt-4">
              <label
                htmlFor="solaris-auth-token-input"
                className="mb-1.5 block text-[11px] font-medium uppercase tracking-wider text-areia/60"
              >
                Engine Access Token
              </label>
              <div className="relative">
                <KeyRound className="pointer-events-none absolute top-1/2 left-3 h-4 w-4 -translate-y-1/2 text-areia/40" />
                <input
                  id="solaris-auth-token-input"
                  type="password"
                  autoFocus
                  autoComplete="current-password"
                  value={authTokenInput}
                  onChange={(e) => setAuthTokenInput(e.target.value)}
                  placeholder="Enter SOLARIS_AUTH_TOKEN"
                  className="w-full rounded-xl border border-areia/15 bg-grafite/80 py-2.5 pr-3 pl-9 text-sm text-areia placeholder:text-areia/35 focus:border-laranja-criacao focus:outline-none"
                />
              </div>
            </div>
            {authError && (
              <div className="mt-3 rounded-lg border border-vermelho-foco/45 bg-vermelho-foco/15 px-3 py-2 text-xs text-areia">
                {authError}
              </div>
            )}
            <div className="mt-5 flex justify-end">
              <button
                type="submit"
                disabled={authSubmitting || !authTokenInput.trim()}
                className="btn-solar-outline rounded-full px-5 py-2 text-xs font-medium text-areia transition-opacity hover:opacity-95 disabled:opacity-40 motion-reduce:transition-none"
              >
                {authSubmitting ? "Authenticating..." : "Unlock Studio"}
              </button>
            </div>
          </form>
        </div>
      )}
    </>
  );
}

export default App;
