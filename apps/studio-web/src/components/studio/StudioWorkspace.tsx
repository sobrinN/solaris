import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Activity,
  Cpu,
  Download,
  Palette,
  Plus,
  Redo2,
  Terminal,
  Undo2,
  Users,
  X,
} from "lucide-react";
import type {
  AgentRole,
  AgentStatusResponse,
  AspectPreset,
  AudioSettings,
  BrandPlaybook,
  CaptionBlock,
  CaptionPreset,
  Clip,
  ColorGrade,
  EngineEventEnvelope,
  ExportJobStatus,
  LogEntry,
  MediaAsset,
  ParsedCubeLut,
  Project,
  ProjectSummary,
  QaReport,
  SavedCodeAsset,
  SolarisEngineClient,
  SwarmRunStatus,
  TimelineCommand,
  Transform2D,
} from "@solaris/engine-client";
import { escapeHtml } from "@solaris/gfx-runtime";
import { SolarisAudioEngine } from "../../lib/audioEngine";
import {
  buildBatchDelete,
  buildBatchSplitAtPlayhead,
  buildGroupMoveBatch,
  lockedClipCount,
  resolveSelection,
} from "../../lib/timelineSelection";
import { SolarisWordmark } from "../SolarisBrand";
import {
  BroadcastScopes,
  type BroadcastScopesHandle,
  type SampledPixelFrame,
} from "./BroadcastScopes";
import { ExportModal } from "./ExportModal";
import { InspectorPanel } from "./InspectorPanel";
import { LogsDrawer } from "./LogsDrawer";
import { ProjectManagerPopover } from "./ProjectManagerPopover";
import { SourceMonitor } from "./SourceMonitor";
import {
  SwarmActivityPanel,
  type ActivityEntry,
  type StartSwarmOptions,
} from "./SwarmActivityPanel";
import { SwarmCursorLayer } from "./SwarmCursorLayer";
import { TimelinePanel } from "./TimelinePanel";
import { useGfxAssetSync } from "./useGfxAssetSync";
import { WebGLProgramStage, type StageVisualLayer } from "./WebGLProgramStage";

const EMPTY_ROLES: AgentRole[] = [];

interface StudioWorkspaceProps {
  client: SolarisEngineClient;
  ticketVersion?: number;
  project: Project;
  projects: ProjectSummary[];
  activeBrand: BrandPlaybook | null;
  assets: SavedCodeAsset[];
  events: EngineEventEnvelope[];
  logs: LogEntry[];
  onClearLogs: () => Promise<void>;
  wsConnected: boolean;
  exportJob: ExportJobStatus | null;
  agentStatus: AgentStatusResponse | null;
  activeModelLabel?: string;
  selectedModelOverride?: string;
  activeSwarmRun: SwarmRunStatus | null;
  swarmError?: string | null;
  latestQaReport: QaReport | null;
  externalPlayheadSeek: { timeSec: number; seq: number } | null;
  modalsOpen?: boolean;
  onSwarmRunChange: (run: SwarmRunStatus | null) => void;
  onClearSwarmError?: () => void;
  onQaReportChange: (report: QaReport | null) => void;
  onExportJobChange: (job: ExportJobStatus | null) => void;
  onAssetCreated: (asset: SavedCodeAsset) => void;
  onAssetsReplaced: (assets: SavedCodeAsset[]) => void;
  onProjectUpdate: (project: Project) => void;
  onSelectProject: (projectId: string) => Promise<void>;
  onCreateBlankProject: (name: string, aspect: AspectPreset) => Promise<void>;
  onRenameProject: (name: string) => Promise<void>;
  onBackToChat: () => void;
  onOpenBrandDrawer: () => void;
  onOpenAiSettings: () => void;
}

function arrayBufferToBase64(buf: ArrayBuffer): string {
  const bytes = new Uint8Array(buf);
  let bin = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    bin += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(bin);
}

export function StudioWorkspace({
  client,
  ticketVersion = 0,
  project,
  projects,
  activeBrand,
  assets,
  events,
  logs,
  onClearLogs,
  wsConnected,
  exportJob,
  agentStatus,
  activeModelLabel,
  selectedModelOverride,
  activeSwarmRun,
  swarmError,
  latestQaReport,
  externalPlayheadSeek,
  modalsOpen = false,
  onSwarmRunChange,
  onClearSwarmError,
  onQaReportChange,
  onExportJobChange,
  onAssetCreated,
  onAssetsReplaced,
  onProjectUpdate,
  onSelectProject,
  onCreateBlankProject,
  onRenameProject,
  onBackToChat,
  onOpenBrandDrawer,
  onOpenAiSettings,
}: StudioWorkspaceProps) {
  const [playheadSec, setPlayheadSec] = useState(0);
  const [isPlaying, setIsPlaying] = useState(false);
  const [swarmCursorsEnabled, setSwarmCursorsEnabled] = useState(
    () =>
      activeSwarmRun !== null &&
      (activeSwarmRun.state === "queued" ||
        activeSwarmRun.state === "planning" ||
        activeSwarmRun.state === "executing_spine" ||
        activeSwarmRun.state === "executing_specialists" ||
        activeSwarmRun.state === "running_qa"),
  );
  const wasSwarmActiveRef = useRef(false);
  const [selectedClipIds, setSelectedClipIds] = useState<string[]>([]);
  const [selectionAnchorId, setSelectionAnchorId] = useState<string | null>(null);
  const [projectManagerOpen, setProjectManagerOpen] = useState(false);
  const [scopesOpen, setScopesOpen] = useState(true);

  useEffect(() => {
    setSelectedClipIds([]);
    setSelectionAnchorId(null);
    setViewerMode("program");
  }, [project.id]);
  const [exportModalOpen, setExportModalOpen] = useState(false);
  const [logsDrawerOpen, setLogsDrawerOpen] = useState(false);
  const [brandLut, setBrandLut] = useState<ParsedCubeLut | null>(null);
  const [brandFontCss, setBrandFontCss] = useState<string>("");
  const [opErrorToast, setOpErrorToast] = useState<string | null>(null);
  const [statusToast, setStatusToast] = useState<string | null>(null);
  const [qaRunning, setQaRunning] = useState(false);
  const [qaError, setQaError] = useState<string | null>(null);
  const [viewerMode, setViewerMode] = useState<"program" | "source">("program");
  const [liveAudioMeters, setLiveAudioMeters] = useState<{
    voicePct: number;
    musicPct: number;
    momentaryLufs: number | null;
  }>({ voicePct: 0, musicPct: 0, momentaryLufs: null });

  const scopesRef = useRef<BroadcastScopesHandle | null>(null);
  const audioEngineRef = useRef<SolarisAudioEngine | null>(null);
  const toastTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const statusToastTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const exportModalOpenRef = useRef(false);
  exportModalOpenRef.current = exportModalOpen;
  const activeVisualLayersCacheRef = useRef<{
    sig: string;
    layers: StageVisualLayer[];
  }>({ sig: "", layers: [] });

  const showErrorToast = useCallback((err: unknown, fallback: string) => {
    const msg = typeof err === "string" ? err : err instanceof Error ? err.message : fallback;
    setOpErrorToast(msg);
    if (toastTimerRef.current) clearTimeout(toastTimerRef.current);
    toastTimerRef.current = setTimeout(() => setOpErrorToast(null), 5000);
  }, []);

  const showStatusToast = useCallback((message: string) => {
    setStatusToast(message);
    if (statusToastTimerRef.current) clearTimeout(statusToastTimerRef.current);
    statusToastTimerRef.current = setTimeout(() => setStatusToast(null), 6000);
  }, []);

  const handleExportDelivered = useCallback(
    (savedPath: string, browserDownload: boolean) => {
      if (exportModalOpenRef.current) return;
      const message = browserDownload
        ? `Downloaded to ${savedPath}`
        : `Rendered video saved to ${savedPath}`;
      showStatusToast(message);
    },
    [showStatusToast],
  );

  useEffect(() => {
    return () => {
      if (statusToastTimerRef.current) clearTimeout(statusToastTimerRef.current);
    };
  }, []);

  useGfxAssetSync({
    client,
    events,
    onAssets: onAssetsReplaced,
    onError: (message) => showErrorToast(message, "GFX library refresh failed"),
  });

  useEffect(() => {
    const engine = new SolarisAudioEngine();
    audioEngineRef.current = engine;
    return () => {
      engine.dispose();
      audioEngineRef.current = null;
      if (toastTimerRef.current) clearTimeout(toastTimerRef.current);
    };
  }, []);

  useEffect(() => {
    if (externalPlayheadSeek !== null && Number.isFinite(externalPlayheadSeek.timeSec)) {
      setIsPlaying(false);
      setPlayheadSec(Math.max(0, externalPlayheadSeek.timeSec));
    }
  }, [externalPlayheadSeek]);

  // Pause timeline playback & Web Audio whenever previewing raw clips in Source Monitor
  useEffect(() => {
    if (viewerMode === "source") {
      setIsPlaying(false);
    }
  }, [viewerMode]);

  // Load Brand Playbook uploaded custom fonts (.woff2/.ttf/.otf) as data URI @font-face CSS for sandboxed GFX iframes
  useEffect(() => {
    const fontAssets = (activeBrand?.reference_media ?? []).filter(
      (r) => r.kind === "font",
    );
    if (!activeBrand || fontAssets.length === 0) {
      setBrandFontCss("");
      return;
    }
    let cancelled = false;
    const loadFonts = async () => {
      const rules: string[] = [];
      for (const f of fontAssets.slice(0, 4)) {
        try {
          const url = client.getBrandMediaStreamUrl(activeBrand.id, f.id);
          const resp = await fetch(url, { credentials: "include" });
          if (!resp.ok) continue;
          const buf = await resp.arrayBuffer();
          if (buf.byteLength === 0 || buf.byteLength > 6 * 1024 * 1024) continue;
          const ext = f.name.split(".").pop()?.toLowerCase() ?? "woff2";
          const mime =
            ext === "woff2"
              ? "font/woff2"
              : ext === "woff"
                ? "font/woff"
                : ext === "otf"
                  ? "font/otf"
                  : "font/ttf";
          const fmt =
            ext === "woff2"
              ? "woff2"
              : ext === "woff"
                ? "woff"
                : ext === "otf"
                  ? "opentype"
                  : "truetype";
          const family = f.name
            .replace(/\.[^.]+$/, "")
            .replace(/[^a-zA-Z0-9 _-]/g, "")
            .trim();
          if (!family) continue;
          const b64 = arrayBufferToBase64(buf);
          rules.push(
            `@font-face { font-family: "${family}"; src: url("data:${mime};base64,${b64}") format("${fmt}"); font-display: block; }`,
          );
        } catch {
          // Skip unavailable font asset
        }
      }
      if (!cancelled) setBrandFontCss(rules.join("\n"));
    };
    void loadFonts();
    return () => {
      cancelled = true;
    };
  }, [client, activeBrand]);

  const totalDurationSec = useMemo(() => {
    let max = 0;
    for (const track of project.tracks) {
      for (const clip of track.clips) {
        max = Math.max(max, clip.start_sec + clip.duration_sec);
      }
    }
    return max > 0 ? max : 16;
  }, [project.tracks]);

  // Smooth rAF playback loop
  useEffect(() => {
    if (!isPlaying) return;
    let rafId = 0;
    let lastTime = performance.now();
    const step = (now: number) => {
      const dt = Math.min(0.1, (now - lastTime) / 1000);
      lastTime = now;
      setPlayheadSec((prev) => {
        const next = prev + dt;
        return next >= totalDurationSec ? 0 : next;
      });
      rafId = requestAnimationFrame(step);
    };
    rafId = requestAnimationFrame(step);
    return () => cancelAnimationFrame(rafId);
  }, [isPlaying, totalDurationSec]);

  // Sync Web Audio Engine to timeline playhead & active tracks
  useEffect(() => {
    audioEngineRef.current?.sync({
      tracks: project.tracks,
      mediaPool: project.media_pool,
      playheadSec,
      isPlaying,
      getStreamUrl: (mediaId) => client.getMediaStreamUrl(project.id, mediaId),
    });
  }, [client, project.id, project.tracks, project.media_pool, playheadSec, isPlaying, ticketVersion]);

  // Sample real Web Audio AnalyserNode levels during playback for BroadcastScopes
  useEffect(() => {
    if (!isPlaying) {
      setLiveAudioMeters({ voicePct: 0, musicPct: 0, momentaryLufs: null });
      return;
    }
    const timer = setInterval(() => {
      const levels = audioEngineRef.current?.getLiveLevels();
      if (levels) setLiveAudioMeters(levels);
    }, 90);
    return () => clearInterval(timer);
  }, [isPlaying]);

  const activityLog = useMemo<ActivityEntry[]>(() => {
    const fromProject: ActivityEntry[] = (project.activity_log ?? []).map(
      (entry) => ({
        id: `rev-${entry.revision}`,
        agentId: entry.agent_id,
        role: entry.role,
        summary: entry.summary,
        timecode: entry.timestamp.slice(11, 19),
      }),
    );

    const fromEvents: ActivityEntry[] = [];
    for (let i = 0; i < events.length; i++) {
      const env = events[i];
      if (env.payload.event === "op_committed") {
        const id = `rev-${env.payload.revision}`;
        if (!fromProject.some((p) => p.id === id)) {
          fromEvents.push({
            id,
            agentId: env.payload.agent_id,
            role: env.payload.role,
            summary: env.payload.summary,
            timecode: env.timestamp.slice(11, 19),
          });
        }
      }
    }

    return [...fromProject, ...fromEvents].reverse().slice(0, 40);
  }, [project.activity_log, events]);

  const swarmRoleCounts = useMemo(() => {
    const counts: Record<string, number> = {};
    for (const item of activityLog) {
      counts[item.role] = (counts[item.role] ?? 0) + 1;
    }
    return counts;
  }, [activityLog]);

  const selectedMedia = useMemo(
    () => project.media_pool[0] ?? null,
    [project.media_pool],
  );

  const selectedClips = useMemo(
    () => resolveSelection(project.tracks, selectedClipIds),
    [project.tracks, selectedClipIds],
  );

  const selectedClip = useMemo(() => {
    if (selectedClips.length !== 1) return null;
    return { trackId: selectedClips[0].trackId, clip: selectedClips[0].clip };
  }, [selectedClips]);

  const hasTimelineClips = useMemo(
    () => project.tracks.some((t) => t.clips.length > 0),
    [project.tracks],
  );

  // Honor track.solo across visual tracks + exclusive end boundary `playheadSec < start + dur`
  const anyVisualSolo = useMemo(
    () =>
      project.tracks.some(
        (t) =>
          (t.id === "v1" || t.id === "v2" || t.id === "g1" || t.id === "t1") && t.solo,
      ),
    [project.tracks],
  );

  const isVisualTrackActive = useCallback(
    (trackId: string) => {
      const t = project.tracks.find((tr) => tr.id === trackId);
      if (!t || t.muted) return false;
      return !anyVisualSolo || t.solo;
    },
    [project.tracks, anyVisualSolo],
  );

  const activeGfxClips = useMemo(() => {
    if (!isVisualTrackActive("g1")) return [];
    const g1 = project.tracks.find((t) => t.id === "g1");
    if (!g1) return [];
    return g1.clips.filter(
      (c) => playheadSec >= c.start_sec && playheadSec < c.start_sec + c.duration_sec,
    );
  }, [project.tracks, playheadSec, isVisualTrackActive]);

  const activeCaptionClip = useMemo(() => {
    if (!isVisualTrackActive("t1")) return null;
    const t1 = project.tracks.find((t) => t.id === "t1");
    if (!t1) return null;
    return (
      t1.clips.find(
        (c) => playheadSec >= c.start_sec && playheadSec < c.start_sec + c.duration_sec,
      ) ?? null
    );
  }, [project.tracks, playheadSec, isVisualTrackActive]);

  // Composite V1 (bottom) and V2 (top); memoize by active clip signature so 60fps playhead ticks keep stable array identity
  const activeVisualLayers = useMemo<StageVisualLayer[]>(() => {
    const layers: StageVisualLayer[] = [];
    const sigParts: string[] = [String(ticketVersion)];
    for (const trackId of ["v1", "v2"] as const) {
      if (!isVisualTrackActive(trackId)) continue;
      const track = project.tracks.find((t) => t.id === trackId);
      const clip = track?.clips.find(
        (c) => playheadSec >= c.start_sec && playheadSec < c.start_sec + c.duration_sec,
      );
      if (clip && clip.payload.kind === "media") {
        const mediaId = clip.payload.media_id;
        const found = project.media_pool.find((m) => m.id === mediaId);
        if (found) {
          const url = found.path ? client.getMediaStreamUrl(project.id, found.id) : null;
          const tr = clip.transform;
          const rf = tr.smart_reframe;
          sigParts.push(
            `${trackId}:${clip.id}:${url ?? ""}:${clip.start_sec}:${clip.duration_sec}:${clip.source_in_sec}:${clip.payload.ken_burns ? 1 : 0}:${tr.x}:${tr.y}:${tr.scale}:${tr.rotation_deg}:${tr.opacity}:${rf ? `${rf.x},${rf.y},${rf.width},${rf.height}` : ""}`,
          );
          layers.push({
            trackId,
            clip,
            media: found,
            url,
          });
        }
      }
    }
    const sig = sigParts.join("|");
    if (sig === activeVisualLayersCacheRef.current.sig) {
      return activeVisualLayersCacheRef.current.layers;
    }
    activeVisualLayersCacheRef.current = { sig, layers };
    return layers;
  }, [
    client,
    project.id,
    project.tracks,
    project.media_pool,
    playheadSec,
    isVisualTrackActive,
    ticketVersion,
  ]);

  // Identify active FX adjustment clip (selected FX grade clip takes priority, else playhead-active unmuted FX clip)
  const playheadFxClip = useMemo<Clip | null>(() => {
    const fx = project.tracks.find((t) => t.id === "fx" && !t.muted);
    return (
      fx?.clips.find(
        (c) =>
          playheadSec >= c.start_sec &&
          playheadSec < c.start_sec + c.duration_sec &&
          c.payload.kind === "grade",
      ) ?? null
    );
  }, [project.tracks, playheadSec]);

  const activeFxClip = useMemo<Clip | null>(() => {
    if (selectedClip?.trackId === "fx" && selectedClip.clip.payload.kind === "grade") {
      return selectedClip.clip;
    }
    return playheadFxClip;
  }, [selectedClip, playheadFxClip]);

  const targetSourceGrade = useMemo<ColorGrade>(() => {
    if (activeFxClip && activeFxClip.payload.kind === "grade") {
      return activeFxClip.payload.grade;
    }
    return project.active_grade;
  }, [activeFxClip, project.active_grade]);

  const gradeTargetId = activeFxClip?.id ?? "master";
  const [draftGradeState, setDraftGradeState] = useState<{
    targetId: string;
    sourceRef: ColorGrade;
    grade: ColorGrade;
  }>({
    targetId: gradeTargetId,
    sourceRef: targetSourceGrade,
    grade: targetSourceGrade,
  });

  const draftGrade =
    draftGradeState.targetId === gradeTargetId &&
    draftGradeState.sourceRef === targetSourceGrade
      ? draftGradeState.grade
      : targetSourceGrade;

  const setDraftGrade = useCallback(
    (next: ColorGrade) => {
      setDraftGradeState({
        targetId: gradeTargetId,
        sourceRef: targetSourceGrade,
        grade: next,
      });
    },
    [gradeTargetId, targetSourceGrade],
  );

  // Effective stage grade: stable reference across 60fps playback ticks, immediately reflects live draftGrade previews
  const effectiveStageGrade = useMemo<ColorGrade>(() => {
    if (!draftGrade.lut_path && activeBrand?.default_lut) {
      return { ...draftGrade, lut_path: activeBrand.default_lut };
    }
    return draftGrade;
  }, [draftGrade, activeBrand?.default_lut]);

  // Resolve 3D LUT from effectiveStageGrade (supports custom FX clip LUTs + brand default_lut)
  useEffect(() => {
    const activeLutPath = effectiveStageGrade.lut_path ?? activeBrand?.default_lut;
    if (!activeBrand || !activeLutPath) {
      setBrandLut(null);
      return;
    }
    let cancelled = false;
    void client
      .getBrandLut(activeBrand.id)
      .then((lut) => {
        if (!cancelled) setBrandLut(lut);
      })
      .catch(() => {
        if (!cancelled) setBrandLut(null);
      });
    return () => {
      cancelled = true;
    };
  }, [client, activeBrand?.id, activeBrand?.default_lut, effectiveStageGrade.lut_path]);

  // Drive BroadcastScopes loudness & meters from live Web Audio AnalyserNode during playback,
  // falling back to active clip levels + measured LUFS when paused.
  const { integratedLufs, voiceLevelPct, musicLevelPct } = useMemo(() => {
    const lufsValues: number[] = [];
    for (const m of project.media_pool) {
      if (typeof m.intelligence.integrated_lufs === "number") {
        lufsValues.push(m.intelligence.integrated_lufs);
      }
    }
    const avgLufs =
      lufsValues.length > 0
        ? lufsValues.reduce((acc, v) => acc + v, 0) / lufsValues.length
        : null;

    if (isPlaying && (liveAudioMeters.voicePct > 0 || liveAudioMeters.musicPct > 0)) {
      return {
        integratedLufs: liveAudioMeters.momentaryLufs ?? avgLufs,
        voiceLevelPct: liveAudioMeters.voicePct,
        musicLevelPct: liveAudioMeters.musicPct,
      };
    }

    const a1 = project.tracks.find((t) => t.id === "a1");
    const v1 = project.tracks.find((t) => t.id === "v1");
    const a2 = project.tracks.find((t) => t.id === "a2");
    const audioSolo = [a1, v1, a2].some((t) => t?.solo);
    const audible = (t: typeof a1) => Boolean(t && !t.muted && (!audioSolo || t.solo));

    const voiceClip = audible(a1)
      ? a1?.clips.find(
          (c) =>
            playheadSec >= c.start_sec &&
            playheadSec < c.start_sec + c.duration_sec &&
            c.audio.gain_db > -80,
        )
      : undefined;
    const v1Clip = audible(v1)
      ? v1?.clips.find(
          (c) =>
            playheadSec >= c.start_sec &&
            playheadSec < c.start_sec + c.duration_sec &&
            c.audio.gain_db > -80,
        )
      : undefined;
    const musicClip = audible(a2)
      ? a2?.clips.find(
          (c) =>
            playheadSec >= c.start_sec &&
            playheadSec < c.start_sec + c.duration_sec &&
            c.audio.gain_db > -80,
        )
      : undefined;

    const dbToPct = (db: number) => Math.max(0, Math.min(100, ((db + 36) / 36) * 100));
    const speechClip = voiceClip ?? v1Clip;
    const vPct = speechClip ? dbToPct(speechClip.audio.gain_db) : 0;
    const mPct = musicClip
      ? dbToPct(
          musicClip.audio.gain_db +
            (speechClip && musicClip.audio.ducking_enabled ? -8 : 0),
        )
      : 0;

    return { integratedLufs: avgLufs, voiceLevelPct: vPct, musicLevelPct: mPct };
  }, [project.tracks, project.media_pool, playheadSec, isPlaying, liveAudioMeters]);

  const handlePixelFrame = useCallback((frame: SampledPixelFrame | null) => {
    scopesRef.current?.drawPixelFrame(frame);
  }, []);

  const handleTogglePlay = useCallback(() => {
    audioEngineRef.current?.unlock();
    setViewerMode("program");
    setIsPlaying((prev) => {
      if (!prev && playheadSec >= totalDurationSec) {
        setPlayheadSec(0);
      }
      return !prev;
    });
  }, [playheadSec, totalDurationSec]);

  const handleSeekPlayhead = useCallback((sec: number) => {
    audioEngineRef.current?.unlock();
    setViewerMode("program");
    setPlayheadSec(Math.max(0, sec));
  }, []);

  const handleUndo = useCallback(async () => {
    try {
      const updated = await client.undo(project.id);
      onProjectUpdate(updated);
    } catch {
      // Nothing to undo
    }
  }, [client, project.id, onProjectUpdate]);

  const handleRedo = useCallback(async () => {
    try {
      const updated = await client.redo(project.id);
      onProjectUpdate(updated);
    } catch {
      // Nothing to redo
    }
  }, [client, project.id, onProjectUpdate]);

  const handleCommitGrade = async (grade: ColorGrade) => {
    setDraftGrade(grade);
    try {
      const res = await client.applyOp(
        project.id,
        activeFxClip
          ? {
              type: "set_color_grade",
              grade,
              track_id: "fx",
              clip_id: activeFxClip.id,
            }
          : { type: "set_color_grade", grade },
        { agent_id: "user", role: "user" },
      );
      onProjectUpdate(res.project);
    } catch (err) {
      showErrorToast(err, "Failed to update color grade (FX track may be locked)");
    }
  };

  const handleToggleTrackFlag = async (
    trackId: string,
    field: "muted" | "solo" | "locked",
    value: boolean,
  ) => {
    try {
      const res = await client.applyOp(
        project.id,
        {
          type: "set_track_flags",
          track_id: trackId,
          [field]: value,
        },
        { agent_id: "user", role: "user" },
      );
      onProjectUpdate(res.project);
    } catch (err) {
      showErrorToast(err, "Failed to toggle track flag");
    }
  };

  const handleAddMediaToTimeline = async (
    item: MediaAsset,
    targetTrackId?: string,
    atSec?: number,
    customRange?: { sourceIn: number; duration: number },
  ) => {
    if (item.kind === "document") {
      showErrorToast(
        new Error("Documents are reference briefs for the AI Director and cannot be placed on timeline tracks."),
        "Cannot place document on timeline",
      );
      return;
    }
    const defaultTrack =
      item.kind === "audio"
        ? item.role_hint === "voice" || item.role_hint === "voiceover"
          ? "a1"
          : "a2"
        : item.role_hint === "b_roll"
          ? "v2"
          : "v1";
    const trackId = targetTrackId ?? defaultTrack;
    const startSec = Number(Math.max(0, atSec ?? playheadSec).toFixed(2));
    const sourceIn = customRange ? Math.max(0, customRange.sourceIn) : 0;
    const naturalDur =
      item.intelligence.duration_sec > 0.2
        ? item.intelligence.duration_sec
        : item.kind === "image"
          ? 3.0
          : 5.0;
    const durationSec = Number(
      Math.max(0.25, customRange ? customRange.duration : naturalDur).toFixed(2),
    );
    const clipId = `clip-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;

    try {
      const res = await client.applyOp(
        project.id,
        {
          type: "place_clip",
          track_id: trackId,
          clip: {
            id: clipId,
            label: item.name,
            start_sec: startSec,
            duration_sec: durationSec,
            source_in_sec: Number(sourceIn.toFixed(2)),
            transform: {
              x: 0,
              y: 0,
              scale: 1,
              rotation_deg: 0,
              opacity: 1,
              smart_reframe:
                project.canvas.aspect_preset === "vertical9x16"
                  ? item.intelligence.default_reframe ?? null
                  : null,
            },
            audio: {
              gain_db: trackId === "a2" ? -14 : trackId === "v2" ? -99 : 0,
              ducking_enabled: trackId === "a2",
              duck_target_db: -18,
              fade_in_sec: trackId === "a2" ? 0.25 : 0,
              fade_out_sec: trackId === "a2" ? 0.6 : 0,
            },
            payload: {
              kind: "media",
              media_id: item.id,
              ken_burns: item.kind === "image",
            },
          },
        },
        { agent_id: "user", role: "user", reason: `added ${item.name} to ${trackId}` },
      );
      onProjectUpdate(res.project);
      setSelectedClipIds([clipId]);
      setSelectionAnchorId(clipId);
    } catch (err) {
      showErrorToast(err, `Failed to place '${item.name}' on ${trackId.toUpperCase()}`);
    }
  };

  const handleTrimOrMoveClip = async (
    trackId: string,
    clipId: string,
    patch: {
      new_start_sec?: number;
      new_duration_sec?: number;
      new_source_in_sec?: number;
    },
  ) => {
    try {
      const res = await client.applyOp(
        project.id,
        {
          type: "trim_clip",
          track_id: trackId,
          clip_id: clipId,
          ...patch,
        },
        { agent_id: "user", role: "user" },
      );
      onProjectUpdate(res.project);
    } catch (err) {
      showErrorToast(err, "Failed to move/trim clip (track may be locked)");
    }
  };

  const handleUpdateClipTransform = async (
    trackId: string,
    clipId: string,
    transform: Transform2D,
  ) => {
    try {
      const res = await client.applyOp(
        project.id,
        {
          type: "update_clip_transform",
          track_id: trackId,
          clip_id: clipId,
          transform,
        },
        { agent_id: "user", role: "user" },
      );
      onProjectUpdate(res.project);
    } catch (err) {
      showErrorToast(err, "Failed to update clip transform");
    }
  };

  const handleUpdateClipAudio = async (
    trackId: string,
    clipId: string,
    audio: AudioSettings,
  ) => {
    try {
      const res = await client.applyOp(
        project.id,
        {
          type: "set_audio_settings",
          track_id: trackId,
          clip_id: clipId,
          audio,
        },
        { agent_id: "user", role: "user" },
      );
      onProjectUpdate(res.project);
    } catch (err) {
      showErrorToast(err, "Failed to update audio settings");
    }
  };

  const handleUpdateCaptionClip = async (
    trackId: string,
    clipId: string,
    caption: CaptionBlock,
  ) => {
    try {
      const res = await client.applyOp(
        project.id,
        {
          type: "update_caption_clip",
          track_id: trackId,
          clip_id: clipId,
          caption,
        },
        { agent_id: "user", role: "user" },
      );
      onProjectUpdate(res.project);
    } catch (err) {
      showErrorToast(err, "Failed to update caption");
    }
  };

  const handleApplyBatchCommand = useCallback(
    async (command: TimelineCommand, lockedSkipped = 0) => {
      try {
        const res = await client.applyOp(project.id, command, {
          agent_id: "user",
          role: "user",
        });
        onProjectUpdate(res.project);
        if (lockedSkipped > 0) {
          showStatusToast(
            `Skipped ${lockedSkipped} locked ${lockedSkipped === 1 ? "clip" : "clips"}.`,
          );
        }
      } catch (err) {
        showErrorToast(err, "Failed to apply batch edit (a track may be locked)");
      }
    },
    [client, onProjectUpdate, project.id, showErrorToast, showStatusToast],
  );

  const handleGroupMove = useCallback(
    async (deltaSec: number) => {
      const cmd = buildGroupMoveBatch(selectedClips, deltaSec);
      const skipped = lockedClipCount(selectedClips);
      if (!cmd) {
        if (skipped > 0) {
          showStatusToast(`Skipped ${skipped} locked ${skipped === 1 ? "clip" : "clips"}.`);
        }
        return;
      }
      await handleApplyBatchCommand(cmd, skipped);
    },
    [handleApplyBatchCommand, selectedClips, showStatusToast],
  );

  const handleSplitSelectedClip = useCallback(async () => {
    if (selectedClips.length === 0) return;
    let seq = 0;
    const stamp = Date.now().toString(36);
    const cmd = buildBatchSplitAtPlayhead(
      selectedClips,
      playheadSec,
      () => `clip_r_${stamp}_${seq++}`,
    );
    const skipped = lockedClipCount(selectedClips);
    if (!cmd) {
      if (skipped > 0) {
        showStatusToast(`Skipped ${skipped} locked ${skipped === 1 ? "clip" : "clips"}.`);
      }
      return;
    }
    await handleApplyBatchCommand(cmd, skipped);
  }, [handleApplyBatchCommand, playheadSec, selectedClips, showStatusToast]);

  const handleDeleteSelectedClip = useCallback(async () => {
    if (selectedClips.length === 0) return;
    const cmd = buildBatchDelete(selectedClips);
    const skipped = lockedClipCount(selectedClips);
    if (!cmd) {
      if (skipped > 0) {
        showStatusToast(`Skipped ${skipped} locked ${skipped === 1 ? "clip" : "clips"}.`);
      }
      return;
    }
    await handleApplyBatchCommand(cmd, skipped);
    setSelectedClipIds((prev) =>
      prev.filter((id) => selectedClips.some((s) => s.clip.id === id && s.locked)),
    );
    setSelectionAnchorId(null);
  }, [handleApplyBatchCommand, selectedClips, showStatusToast]);

  // Global keyboard shortcuts (disabled when modals/drawers are open or interactive elements are focused)
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (exportModalOpen || logsDrawerOpen || projectManagerOpen || modalsOpen) return;
      const target = e.target as HTMLElement | null;
      if (
        target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.tagName === "SELECT" ||
          target.tagName === "BUTTON" ||
          target.tagName === "A" ||
          target.getAttribute("role") === "slider" ||
          target.isContentEditable)
      ) {
        return;
      }
      if (e.code === "Space") {
        e.preventDefault();
        handleTogglePlay();
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "z") {
        e.preventDefault();
        if (e.shiftKey) {
          void handleRedo();
        } else {
          void handleUndo();
        }
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "a") {
        e.preventDefault();
        const allIds = project.tracks.flatMap((t) =>
          [...t.clips].sort((a, b) => a.start_sec - b.start_sec).map((c) => c.id),
        );
        setSelectedClipIds(allIds);
        setSelectionAnchorId(allIds[0] ?? null);
      } else if (e.key === "Escape" && selectedClipIds.length > 0) {
        e.preventDefault();
        setSelectedClipIds([]);
        setSelectionAnchorId(null);
      } else if (
        (e.key === "Delete" || e.key === "Backspace") &&
        selectedClipIds.length > 0
      ) {
        e.preventDefault();
        void handleDeleteSelectedClip();
      } else if (
        e.key.toLowerCase() === "s" &&
        !e.metaKey &&
        !e.ctrlKey &&
        selectedClipIds.length > 0
      ) {
        e.preventDefault();
        void handleSplitSelectedClip();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [
    exportModalOpen,
    logsDrawerOpen,
    projectManagerOpen,
    modalsOpen,
    project.tracks,
    selectedClipIds,
    handleTogglePlay,
    handleUndo,
    handleRedo,
    handleDeleteSelectedClip,
    handleSplitSelectedClip,
  ]);

  const handlePlaceSavedAsset = async (
    asset: SavedCodeAsset,
    atSec?: number,
    customDurationSec?: number,
  ) => {
    const clipId = `gfx-${Date.now()}`;
    const startSec = Number(Math.max(0, atSec ?? playheadSec).toFixed(2));
    const durationSec = Number(
      Math.max(0.25, customDurationSec ?? asset.default_duration_sec).toFixed(2),
    );
    try {
      const res = await client.applyOp(
        project.id,
        {
          type: "place_gfx",
          track_id: "g1",
          clip_id: clipId,
          label: asset.name,
          start_sec: startSec,
          duration_sec: durationSec,
          gfx: {
            asset_id: asset.id,
            props: asset.default_props,
            inline_html: asset.html_source,
          },
        },
        { agent_id: "user", role: "user", reason: `placed ${asset.name}` },
      );
      onProjectUpdate(res.project);
      setSelectedClipIds([clipId]);
      setSelectionAnchorId(clipId);
    } catch (err) {
      showErrorToast(err, "Failed to place GFX asset (G1 track may be locked)");
    }
  };

  const handlePlaceGfxFromDrop = (asset: SavedCodeAsset, startSec: number) => {
    const remaining = Math.max(0, totalDurationSec - startSec);
    const durationSec =
      remaining >= 0.25 ? Math.min(4, remaining) : Math.min(4, asset.default_duration_sec);
    void handlePlaceSavedAsset(asset, startSec, durationSec);
  };

  const handleUpdateGfxProps = async (
    trackId: string,
    clipId: string,
    props: Record<string, unknown>,
  ) => {
    try {
      const res = await client.applyOp(
        project.id,
        {
          type: "update_gfx_props",
          track_id: trackId,
          clip_id: clipId,
          props,
        },
        { agent_id: "user", role: "user" },
      );
      onProjectUpdate(res.project);
    } catch (err) {
      showErrorToast(err, "Failed to update GFX props");
    }
  };

  const handleUpdateCaptionPreset = async (preset: CaptionPreset) => {
    const target =
      selectedClip?.clip.payload.kind === "caption"
        ? selectedClip.clip
        : activeCaptionClip;
    if (!target || target.payload.kind !== "caption") return;
    try {
      const res = await client.applyOp(
        project.id,
        {
          type: "update_caption_clip",
          track_id: "t1",
          clip_id: target.id,
          caption: { ...target.payload.caption, preset },
        },
        { agent_id: "user", role: "user" },
      );
      onProjectUpdate(res.project);
    } catch (err) {
      showErrorToast(err, "Failed to update caption preset");
    }
  };

  const handleSaveClipAsAsset = async (clip: Clip) => {
    if (clip.payload.kind !== "gfx") return;
    const html =
      clip.payload.gfx.inline_html ??
      `<div class="clip" data-start="0" data-duration="${clip.duration_sec.toFixed(2)}" style="padding:12px 18px;border-radius:14px;background:rgba(8,4,4,0.85);border:1px solid var(--brand-accent);color:var(--brand-ink);">${escapeHtml(clip.label)}</div>`;
    const props = clip.payload.gfx.props;
    const properties: Record<string, { type: string }> = {};
    for (const [k, v] of Object.entries(props)) {
      properties[k] = {
        type:
          typeof v === "number"
            ? "number"
            : typeof v === "boolean"
              ? "boolean"
              : "string",
      };
    }
    try {
      const saved = await client.saveAsset({
        id: `custom-${Date.now()}`,
        name: `${clip.label} (Saved)`,
        category: "custom",
        tags: ["saved", "custom"],
        props_schema: { type: "object", properties },
        default_props: props,
        html_source: html,
        default_duration_sec: clip.duration_sec,
        created_at: new Date().toISOString(),
      });
      onAssetCreated(saved);
    } catch (err) {
      showErrorToast(err, "Failed to save GFX asset");
    }
  };

  const handleStartSwarm = async (options: StartSwarmOptions) => {
    onClearSwarmError?.();
    const run = await client.startSwarmRun(project.id, {
      prompt: options.prompt,
      mode: options.mode,
      selected_clip_ids: options.selectedClipIds,
      playhead_sec: options.playheadSec,
      conversation_history: options.conversationHistory,
      model_override: selectedModelOverride || undefined,
      allow_local_fallback: !(agentStatus?.openrouter_configured ?? false),
    });
    onSwarmRunChange(run);
  };

  const handleCancelSwarm = async () => {
    if (!activeSwarmRun) return;
    await client.cancelSwarmRun(project.id, activeSwarmRun.run_id);
  };

  const fallbackTokens = useMemo(
    () =>
      activeBrand?.tokens ?? {
        bg: "#080404",
        surface: "#1A0E0E",
        ink: "#F7F5EF",
        accent: "#F05A00",
        accent_secondary: "#F7B733",
        font_display: "Sora",
        font_body: "Sora",
      },
    [activeBrand],
  );

  const aspectBadge =
    project.canvas.aspect_preset === "vertical9x16"
      ? "9:16"
      : project.canvas.aspect_preset === "landscape16x9"
        ? "16:9"
        : "1:1";

  const reframeBoxWidthPct =
    project.canvas.aspect_preset === "vertical9x16"
      ? 31.6
      : project.canvas.aspect_preset === "square1x1"
        ? 56.25
        : 94;

  const projectFps =
    project.canvas.fps_num / Math.max(1, project.canvas.fps_den);

  const isSwarmActive =
    activeSwarmRun !== null &&
    (activeSwarmRun.state === "queued" ||
      activeSwarmRun.state === "planning" ||
      activeSwarmRun.state === "executing_spine" ||
      activeSwarmRun.state === "executing_specialists" ||
      activeSwarmRun.state === "running_qa");

  useEffect(() => {
    if (isSwarmActive && !wasSwarmActiveRef.current) {
      setSwarmCursorsEnabled(true);
    }
    wasSwarmActiveRef.current = isSwarmActive;
  }, [isSwarmActive]);

  const handleRunQa = async () => {
    if (qaRunning) return;
    setQaRunning(true);
    setQaError(null);
    try {
      const report = await client.runProjectQa(project.id, true);
      onQaReportChange(report);
      const updated = await client.getProject(project.id);
      onProjectUpdate(updated);
    } catch (err) {
      setQaError(err instanceof Error ? err.message : "Failed to run QA");
    } finally {
      setQaRunning(false);
    }
  };

  const handleInsertSourceRange = (media: MediaAsset, sourceIn: number, duration: number) => {
    setViewerMode("program");
    void handleAddMediaToTimeline(media, undefined, playheadSec, { sourceIn, duration });
  };

  return (
    <div className="relative flex min-h-dvh flex-col overflow-y-auto bg-void text-areia lg:h-dvh lg:overflow-hidden">
      <SwarmCursorLayer
        enabled={swarmCursorsEnabled}
        swarmActive={isSwarmActive}
        activeRoles={activeSwarmRun?.active_roles ?? EMPTY_ROLES}
        events={events}
        totalDurationSec={totalDurationSec}
      />

      <ExportModal
        open={exportModalOpen}
        projectId={project.id}
        projectName={project.name}
        client={client}
        activeJob={exportJob}
        onJobStarted={(job) => onExportJobChange(job)}
        onExportDelivered={handleExportDelivered}
        onClose={() => setExportModalOpen(false)}
      />

      <LogsDrawer
        open={logsDrawerOpen}
        logs={logs}
        onClear={onClearLogs}
        onClose={() => setLogsDrawerOpen(false)}
      />

      {statusToast && !opErrorToast && (
        <div
          role="status"
          className="fixed right-4 bottom-4 z-50 flex max-w-sm items-center gap-2 rounded-xl border border-amarelo-ideias/45 bg-carbono/95 px-3.5 py-2.5 text-xs text-areia shadow-2xl backdrop-blur-md"
        >
          <span className="flex-1 truncate" title={statusToast}>
            {statusToast}
          </span>
          <button
            type="button"
            onClick={() => setStatusToast(null)}
            className="rounded-full p-0.5 text-areia/65 hover:text-areia"
            aria-label="Dismiss notification"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      )}

      {opErrorToast && (
        <div
          role="alert"
          className="fixed right-4 bottom-4 z-50 flex max-w-sm items-center gap-2 rounded-xl border border-vermelho-foco/55 bg-carbono/95 px-3.5 py-2.5 text-xs text-areia shadow-2xl backdrop-blur-md"
        >
          <span className="flex-1">{opErrorToast}</span>
          <button
            type="button"
            onClick={() => setOpErrorToast(null)}
            className="rounded-full p-0.5 text-areia/65 hover:text-areia"
            aria-label="Dismiss error"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      )}

      {/* Top NLE Chrome — 3-Zone Pro Studio Header */}
      <header className="relative z-30 flex h-13 shrink-0 items-center justify-between gap-3 border-b border-areia/10 bg-carbono/75 px-4 backdrop-blur-md">
        {/* Left Zone: Brand + Project Manager */}
        <div className="flex min-w-0 items-center gap-2.5">
          <SolarisWordmark />
          <div className="hidden h-4 w-px bg-areia/15 sm:block" />
          <ProjectManagerPopover
            currentProject={project}
            projects={projects}
            open={projectManagerOpen}
            onOpenChange={setProjectManagerOpen}
            onSelectProject={onSelectProject}
            onCreateBlankProject={onCreateBlankProject}
            onRenameProject={onRenameProject}
            onNewProjectInChat={onBackToChat}
          />
          <button
            type="button"
            onClick={onBackToChat}
            title="Start a new project with fresh media or prompt"
            className="hidden h-8 items-center gap-1 rounded-full border border-areia/12 bg-void/65 px-2.5 text-xs text-areia/80 transition-colors hover:border-laranja-criacao/50 hover:bg-accent hover:text-areia md:flex motion-reduce:transition-none"
          >
            <Plus className="h-3.5 w-3.5 text-laranja-criacao" />
            <span>New</span>
          </button>
        </div>

        {/* Center Zone: History & Swarm Presence */}
        <div className="hidden items-center gap-2 lg:flex">
          <div className="flex items-center rounded-full border border-areia/12 bg-void/65 p-0.5">
            <button
              type="button"
              onClick={() => void handleUndo()}
              className="grid h-7 w-7 place-items-center rounded-full text-areia/80 hover:bg-accent hover:text-areia"
              title="Undo (⌘Z)"
              aria-label="Undo"
            >
              <Undo2 className="h-3.5 w-3.5" />
            </button>
            <button
              type="button"
              onClick={() => void handleRedo()}
              className="grid h-7 w-7 place-items-center rounded-full text-areia/80 hover:bg-accent hover:text-areia"
              title="Redo (⇧⌘Z)"
              aria-label="Redo"
            >
              <Redo2 className="h-3.5 w-3.5" />
            </button>
          </div>

          <button
            type="button"
            onClick={() => setSwarmCursorsEnabled((v) => !v)}
            title={
              wsConnected
                ? "Swarm WebSocket connected · Toggle agent cursors"
                : "Reconnecting to Solaris Engine"
            }
            className={`flex h-8 items-center gap-2 rounded-full border px-3 text-xs transition-colors motion-reduce:transition-none ${
              swarmCursorsEnabled
                ? "border-laranja-criacao/55 bg-accent text-areia"
                : "border-areia/12 bg-void/65 text-areia/75 hover:bg-accent hover:text-areia"
            }`}
          >
            <span
              className={`h-1.5 w-1.5 rounded-full ${
                wsConnected ? "bg-amarelo-ideias solar-pulse-dot" : "bg-vermelho-foco"
              }`}
            />
            <Users className="h-3.5 w-3.5 text-laranja-criacao" />
            <span>{swarmCursorsEnabled ? "Swarm Live" : "Focus Mode"}</span>
          </button>
        </div>

        {/* Right Zone: Studio Utilities + Primary Export CTA */}
        <div className="flex items-center gap-1.5">
          <div className="flex items-center gap-1">
            <button
              type="button"
              onClick={() => setLogsDrawerOpen(true)}
              className="flex h-8 items-center gap-1.5 rounded-full border border-areia/12 bg-void/55 px-2.5 text-xs text-areia/85 hover:bg-accent xl:px-3"
              title="Open Diagnostic Logs (OpenRouter, Swarm, QA & Export)"
              aria-label="Diagnostic Logs"
            >
              <Terminal className="h-3.5 w-3.5 text-amarelo-ideias" />
              <span className="hidden xl:inline">Logs</span>
              {logs.some((l) => l.level === "error") && (
                <span className="tabular-nums rounded-full bg-vermelho-foco px-1.5 text-[10px] font-semibold text-areia">
                  {logs.filter((l) => l.level === "error").length}
                </span>
              )}
            </button>

            <button
              type="button"
              onClick={onOpenAiSettings}
              className="flex h-8 items-center gap-1.5 rounded-full border border-areia/12 bg-void/55 px-2.5 text-xs text-areia/85 hover:bg-accent xl:px-3"
              title="OpenRouter API Key, Model Advisor & MCP Mode"
              aria-label="AI Settings"
            >
              <Cpu className="h-3.5 w-3.5 text-amarelo-ideias" />
              <span className="hidden xl:inline">
                {agentStatus?.openrouter_configured ? "AI Ready" : "AI Keys"}
              </span>
            </button>

            <button
              type="button"
              data-cursor-anchor="brand.playbook"
              onClick={onOpenBrandDrawer}
              className="flex h-8 items-center gap-1.5 rounded-full border border-areia/12 bg-void/55 px-2.5 text-xs text-areia/85 hover:bg-accent md:px-3"
              title="Open Brand Playbook"
              aria-label="Brand Playbook"
            >
              <Palette className="h-3.5 w-3.5 text-laranja-criacao" />
              <span className="hidden md:inline">Brand</span>
            </button>
          </div>

          <div className="mx-1 hidden h-4 w-px bg-areia/14 sm:block" />

          <button
            type="button"
            onClick={() => setExportModalOpen(true)}
            className="btn-solar-outline flex h-8 items-center gap-1.5 px-4 text-xs font-medium text-areia"
          >
            <Download className="h-3.5 w-3.5 text-amarelo-ideias" />
            <span className="tabular-nums">
              {exportJob &&
              (exportJob.state === "queued" ||
                exportJob.state === "rendering_gfx" ||
                exportJob.state === "compositing")
                ? `Export · ${Math.round(exportJob.progress * 100)}%`
                : "Export"}
            </span>
          </button>
        </div>
      </header>

      {/* Main Studio Grid — Media-First Pro Layout (Director | Hero Monitor | Optional Scopes | Inspector) */}
      <div
        className={`grid min-h-0 flex-1 grid-cols-1 gap-3 p-3 ${
          scopesOpen
            ? "lg:grid-cols-[minmax(300px,340px)_minmax(340px,1fr)_minmax(276px,308px)] xl:grid-cols-[minmax(330px,375px)_minmax(400px,1fr)_212px_minmax(288px,320px)]"
            : "lg:grid-cols-[minmax(320px,360px)_minmax(420px,1fr)_minmax(290px,324px)] xl:grid-cols-[minmax(350px,390px)_minmax(500px,1fr)_minmax(310px,340px)]"
        }`}
      >
        {/* Left Column: AI Director & Swarm Co-Pilot */}
        <div className="flex min-h-0 flex-col">
          <SwarmActivityPanel
            projectId={project.id}
            hasTimelineClips={hasTimelineClips}
            selectedClips={selectedClips}
            playheadSec={playheadSec}
            onRemoveSelectedClip={(clipId) =>
              setSelectedClipIds((prev) => prev.filter((id) => id !== clipId))
            }
            agentStatus={agentStatus}
            activeModelLabel={activeModelLabel}
            activeSwarmRun={activeSwarmRun}
            latestQaReport={latestQaReport}
            qaRunning={qaRunning}
            qaError={qaError}
            swarmError={swarmError}
            activityLog={activityLog}
            logs={logs}
            onClearLogs={onClearLogs}
            onOpenLogsDrawer={() => setLogsDrawerOpen(true)}
            onStartSwarm={handleStartSwarm}
            onCancelSwarm={handleCancelSwarm}
            onRunQa={handleRunQa}
          />
        </div>

        {/* Center Hero Stage: Program Monitor */}
        <div
          data-cursor-anchor="program.stage"
          className="flex min-h-[280px] flex-col rounded-2xl border border-areia/10 bg-carbono/70 p-3 lg:min-h-0"
        >
          <div className="mb-2.5 flex items-center justify-between gap-2 text-[10px] font-semibold uppercase tracking-[0.14em]">
            {project.media_pool.length > 0 ? (
              <div
                role="tablist"
                aria-label="Viewer"
                className="flex items-center gap-0.5 rounded-full border border-areia/10 bg-void/65 p-0.5"
              >
                <button
                  type="button"
                  role="tab"
                  aria-selected={viewerMode === "program"}
                  onClick={() => setViewerMode("program")}
                  className={`rounded-full px-2.5 py-0.5 transition-colors motion-reduce:transition-none ${
                    viewerMode === "program"
                      ? "bg-laranja-criacao/20 text-areia"
                      : "text-areia/55 hover:text-areia"
                  }`}
                >
                  Program
                </button>
                <button
                  type="button"
                  role="tab"
                  aria-selected={viewerMode === "source"}
                  data-cursor-anchor="source.monitor"
                  onClick={() => setViewerMode("source")}
                  className={`rounded-full px-2.5 py-0.5 transition-colors motion-reduce:transition-none ${
                    viewerMode === "source"
                      ? "bg-laranja-criacao/20 text-areia"
                      : "text-areia/55 hover:text-areia"
                  }`}
                >
                  Source
                </button>
              </div>
            ) : (
              <div className="flex items-center gap-2 text-areia/80">
                <span className="h-1.5 w-1.5 rounded-full bg-laranja-criacao" />
                <span>Program</span>
              </div>
            )}

            <div className="flex items-center gap-2">
              <span className="truncate rounded-full border border-areia/10 bg-void/70 px-2.5 py-0.5 tabular-nums text-amarelo-ideias">
                {viewerMode === "program"
                  ? `${aspectBadge} · ${project.canvas.width}×${project.canvas.height}`
                  : (selectedMedia?.name ?? "No clip")}
              </span>
              <button
                type="button"
                onClick={() => setScopesOpen((v) => !v)}
                title={scopesOpen ? "Hide Broadcast Scopes" : "Show Broadcast Scopes"}
                aria-pressed={scopesOpen}
                className={`hidden items-center gap-1 rounded-full border px-2.5 py-0.5 text-[10px] font-medium normal-case tracking-normal transition-colors xl:flex motion-reduce:transition-none ${
                  scopesOpen
                    ? "border-amarelo-ideias/45 bg-amarelo-ideias/12 text-amarelo-ideias"
                    : "border-areia/12 bg-void/65 text-areia/65 hover:bg-accent hover:text-areia"
                }`}
              >
                <Activity className="h-3 w-3" />
                <span>Scopes</span>
              </button>
            </div>
          </div>

          <div
            className={`relative min-h-0 flex-1 items-center justify-center overflow-hidden rounded-xl border border-areia/12 bg-void p-2.5 ${
              viewerMode === "program" ? "flex" : "hidden"
            }`}
          >
            <WebGLProgramStage
              client={client}
              ticketVersion={ticketVersion}
              width={project.canvas.width}
              height={project.canvas.height}
              playheadSec={playheadSec}
              isPlaying={isPlaying}
              grade={effectiveStageGrade}
              activeLayers={activeVisualLayers}
              activeGfxClips={activeGfxClips}
              activeCaptionClip={activeCaptionClip}
              brandTokens={fallbackTokens}
              brandFontCss={brandFontCss}
              brandLut={brandLut}
              onPixelFrame={handlePixelFrame}
            />
          </div>
          {viewerMode === "source" && project.media_pool.length > 0 && (
            <SourceMonitor
              key={selectedMedia?.id ?? "none"}
              media={selectedMedia}
              streamUrl={
                selectedMedia?.path ? client.getMediaStreamUrl(project.id, selectedMedia.id) : null
              }
              aspectBadge={aspectBadge}
              canvasAspect={project.canvas.width / Math.max(1, project.canvas.height)}
              reframeBoxWidthPct={reframeBoxWidthPct}
              onInsertRange={handleInsertSourceRange}
            />
          )}
        </div>

        {/* Optional Broadcast Scopes Column (toggleable on xl+) */}
        {scopesOpen && (
          <div className="hidden min-h-0 xl:block">
            <BroadcastScopes
              scopesRef={scopesRef}
              grade={effectiveStageGrade}
              playheadSec={playheadSec}
              integratedLufs={integratedLufs}
              voiceLevelPct={voiceLevelPct}
              musicLevelPct={musicLevelPct}
            />
          </div>
        )}

        {/* Right: Inspector */}
        <div className="min-h-0">
          <InspectorPanel
            grade={draftGrade}
            gradeTargetLabel={activeFxClip ? "FX Clip" : "Master Grade"}
            hasBrandLut={Boolean(brandLut)}
            onPreviewGrade={setDraftGrade}
            onCommitGrade={(next) => void handleCommitGrade(next)}
            selectedClip={selectedClip}
            selectedClips={selectedClips}
            onApplyBatchCommand={(cmd, lockedSkipped) =>
              void handleApplyBatchCommand(cmd, lockedSkipped)
            }
            onClearSelection={() => {
              setSelectedClipIds([]);
              setSelectionAnchorId(null);
            }}
            activeCaptionPreset={
              activeCaptionClip?.payload.kind === "caption"
                ? activeCaptionClip.payload.caption.preset
                : null
            }
            savedAssets={assets}
            onUpdateGfxProps={(trackId, clipId, props) =>
              void handleUpdateGfxProps(trackId, clipId, props)
            }
            onUpdateClipTransform={(trackId, clipId, tr) =>
              void handleUpdateClipTransform(trackId, clipId, tr)
            }
            onUpdateClipAudio={(trackId, clipId, audio) =>
              void handleUpdateClipAudio(trackId, clipId, audio)
            }
            onUpdateCaptionClip={(trackId, clipId, caption) =>
              void handleUpdateCaptionClip(trackId, clipId, caption)
            }
            onUpdateCaptionPreset={(preset) => void handleUpdateCaptionPreset(preset)}
            onSaveClipAsAsset={handleSaveClipAsAsset}
            swarmRoleCounts={swarmRoleCounts}
          />
        </div>
      </div>

      {/* Bottom: 7-Track Timeline */}
      <div className="shrink-0 px-2.5 pb-2.5">
        <TimelinePanel
          tracks={project.tracks}
          mediaPool={project.media_pool}
          savedAssets={assets}
          playheadSec={playheadSec}
          totalDurationSec={totalDurationSec}
          fps={projectFps}
          canvasLabel={`${aspectBadge} · ${project.canvas.width}×${project.canvas.height}`}
          isPlaying={isPlaying}
          selectedClipIds={selectedClipIds}
          selectionAnchorId={selectionAnchorId}
          onSelectionChange={(ids, anchorId) => {
            setSelectedClipIds(ids);
            setSelectionAnchorId(anchorId);
          }}
          onTogglePlay={handleTogglePlay}
          onSeek={handleSeekPlayhead}
          onSplitSelected={() => void handleSplitSelectedClip()}
          onDeleteSelected={() => void handleDeleteSelectedClip()}
          onGroupMove={(deltaSec) => void handleGroupMove(deltaSec)}
          onTrimOrMoveClip={(trackId, clipId, patch) =>
            void handleTrimOrMoveClip(trackId, clipId, patch)
          }
          onDropMediaOnTrack={(trackId, mediaId, startSec) => {
            const found = project.media_pool.find((m) => m.id === mediaId);
            if (found) {
              void handleAddMediaToTimeline(found, trackId, startSec);
            }
          }}
          onPlaceGfxFromAsset={handlePlaceGfxFromDrop}
          onToggleTrackFlag={(trackId, field, value) =>
            void handleToggleTrackFlag(trackId, field, value)
          }
        />
      </div>
    </div>
  );
}

export default StudioWorkspace;
