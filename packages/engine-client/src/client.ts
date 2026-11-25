import type {
  AgentRole,
  AgentStatusResponse,
  AspectPreset,
  AssetPackImportResponse,
  BrandPlaybook,
  BrandReferenceKind,
  CodeAssetPackManifest,
  CommandMeta,
  CreativePlan,
  EngineEventEnvelope,
  ExportJobStatus,
  ExportRequest,
  GeneratePlanRequest,
  GfxSandboxResponse,
  HealthResponse,
  LogEntry,
  LogLevel,
  MediaAsset,
  ModelAdvisorResponse,
  OpResult,
  ParsedCubeLut,
  Project,
  ProjectSummary,
  ProjectTemplateInfo,
  QaReport,
  SavedCodeAsset,
  SkillSummary,
  SwarmRunRequest,
  SwarmRunStatus,
  TimelineCommand,
  TranscriptWord,
  UpdateProjectMetadataRequest,
} from "./types.js";

export interface EngineClientOptions {
  baseUrl?: string;
  authToken?: string;
}

export class EngineApiError extends Error {
  readonly status: number;
  readonly body: string;

  constructor(status: number, body: string) {
    super(`Solaris Engine ${status}: ${body}`);
    this.name = "EngineApiError";
    this.status = status;
    this.body = body;
  }
}

const MAX_GFX_SANDBOX_CACHE_ENTRIES = 128;

function hashGfxHtml(html: string): string {
  let h1 = 0x811c9dc5;
  let h2 = 0x9e3779b9;
  for (let i = 0; i < html.length; i += 1) {
    const c = html.charCodeAt(i);
    h1 ^= c;
    h1 = Math.imul(h1, 0x01000193);
    h2 ^= c;
    h2 = Math.imul(h2, 0x85ebca6b);
  }
  return `${html.length}:${(h1 >>> 0).toString(36)}:${(h2 >>> 0).toString(36)}`;
}

export class SolarisEngineClient {
  private readonly baseUrl: string;
  private authToken?: string;
  private hasSession = false;
  private mediaTicket?: string;
  private ticketExpiresAt = 0;
  private refreshTimer?: ReturnType<typeof setTimeout>;
  private readonly ticketListeners = new Set<() => void>();
  private readonly gfxSandboxCache = new Map<string, { id: string; html: string }>();

  constructor(options: EngineClientOptions = {}) {
    this.baseUrl = (options.baseUrl ?? "http://127.0.0.1:41731").replace(/\/+$/, "");
    this.authToken = options.authToken;
  }

  getBaseUrl(): string {
    return this.baseUrl;
  }

  onTicketChange(listener: () => void): () => void {
    this.ticketListeners.add(listener);
    return () => {
      this.ticketListeners.delete(listener);
    };
  }

  dispose(): void {
    if (this.refreshTimer) {
      clearTimeout(this.refreshTimer);
      this.refreshTimer = undefined;
    }
    this.ticketListeners.clear();
    this.gfxSandboxCache.clear();
  }

  private headers(extra?: Record<string, string>): Record<string, string> {
    const out: Record<string, string> = { ...extra };
    if (this.authToken) {
      out["Authorization"] = `Bearer ${this.authToken}`;
    }
    return out;
  }

  private async request<T>(path: string, init?: RequestInit): Promise<T> {
    const res = await fetch(`${this.baseUrl}${path}`, {
      ...init,
      credentials: "include",
      headers: this.headers(init?.headers as Record<string, string> | undefined),
    });
    if (!res.ok) {
      const text = await res.text().catch(() => res.statusText);
      throw new EngineApiError(res.status, text);
    }
    if (res.status === 204 || res.headers.get("content-length") === "0") {
      return undefined as T;
    }
    return (await res.json()) as T;
  }

  async signIn(token: string): Promise<void> {
    const trimmed = token.trim();
    const prevToken = this.authToken;
    this.authToken = trimmed || undefined;
    try {
      await this.establishSession(true);
    } catch (err) {
      this.authToken = prevToken;
      throw err;
    }
  }

  async signOut(): Promise<void> {
    this.authToken = undefined;
    this.hasSession = false;
    this.mediaTicket = undefined;
    this.ticketExpiresAt = 0;
    this.gfxSandboxCache.clear();
    if (this.refreshTimer) {
      clearTimeout(this.refreshTimer);
      this.refreshTimer = undefined;
    }
    try {
      await this.request<void>("/api/v1/auth/session", { method: "DELETE" });
    } catch {
      // Ignore if already signed out
    }
    for (const listener of this.ticketListeners) listener();
  }

  async establishSession(force = false): Promise<void> {
    if (!force && !this.authToken && !this.hasSession) return;
    const res = await this.request<{ media_ticket: string; expires_in_sec?: number }>(
      "/api/v1/auth/session",
      { method: "POST" },
    );
    this.hasSession = true;
    this.mediaTicket = res.media_ticket;
    const ttlMs = (res.expires_in_sec ?? 1800) * 1000;
    this.ticketExpiresAt = Date.now() + ttlMs;
    if (this.refreshTimer) clearTimeout(this.refreshTimer);
    this.refreshTimer = setTimeout(() => {
      void this.establishSession(true).catch(() => {});
    }, Math.max(30_000, ttlMs * 0.75));
    for (const listener of this.ticketListeners) listener();
  }

  private appendTicket(url: string): string {
    if (!this.mediaTicket) return url;
    const sep = url.includes("?") ? "&" : "?";
    return `${url}${sep}ticket=${encodeURIComponent(this.mediaTicket)}`;
  }

  health(): Promise<HealthResponse> {
    return this.request<HealthResponse>("/api/v1/health");
  }

  listProjects(): Promise<ProjectSummary[]> {
    return this.request<ProjectSummary[]>("/api/v1/projects");
  }

  listTemplates(): Promise<ProjectTemplateInfo[]> {
    return this.request<ProjectTemplateInfo[]>("/api/v1/templates");
  }

  createProject(
    name: string,
    aspectPreset?: AspectPreset,
    brandPlaybookId?: string,
    prompt?: string,
    templateId?: string,
  ): Promise<Project> {
    const body: Record<string, unknown> = {
      name,
      brand_playbook_id: brandPlaybookId,
      prompt,
      template_id: templateId,
    };
    if (aspectPreset) {
      body.aspect_preset = aspectPreset;
    }
    return this.request<Project>("/api/v1/projects", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
  }

  acquireTrackLock(
    projectId: string,
    trackId: string,
    agentId: string,
    role: AgentRole,
  ): Promise<void> {
    return this.request<void>(
      `/api/v1/projects/${encodeURIComponent(projectId)}/locks/${encodeURIComponent(trackId)}`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ agent_id: agentId, role }),
      },
    );
  }

  releaseTrackLock(
    projectId: string,
    trackId: string,
    agentId: string,
  ): Promise<void> {
    return this.request<void>(
      `/api/v1/projects/${encodeURIComponent(projectId)}/locks/${encodeURIComponent(trackId)}?agent_id=${encodeURIComponent(agentId)}`,
      { method: "DELETE" },
    );
  }

  getProject(projectId: string): Promise<Project> {
    return this.request<Project>(`/api/v1/projects/${encodeURIComponent(projectId)}`);
  }

  updateProjectMetadata(
    projectId: string,
    body: UpdateProjectMetadataRequest,
  ): Promise<Project> {
    return this.request<Project>(`/api/v1/projects/${encodeURIComponent(projectId)}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
  }

  applyOp(
    projectId: string,
    command: TimelineCommand,
    meta: CommandMeta = { agent_id: "user", role: "user" },
  ): Promise<{ result: OpResult; project: Project }> {
    return this.request<{ result: OpResult; project: Project }>(
      `/api/v1/projects/${encodeURIComponent(projectId)}/ops`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ command, meta }),
      },
    );
  }

  undo(projectId: string): Promise<Project> {
    return this.request<Project>(
      `/api/v1/projects/${encodeURIComponent(projectId)}/undo`,
      { method: "POST" },
    );
  }

  redo(projectId: string): Promise<Project> {
    return this.request<Project>(
      `/api/v1/projects/${encodeURIComponent(projectId)}/redo`,
      { method: "POST" },
    );
  }

  importMediaFiles(projectId: string, files: File[]): Promise<MediaAsset[]> {
    const form = new FormData();
    for (const file of files) {
      form.append("file", file, file.name);
    }
    return this.request<MediaAsset[]>(
      `/api/v1/projects/${encodeURIComponent(projectId)}/media/import`,
      {
        method: "POST",
        body: form,
      },
    );
  }

  importMediaPaths(
    projectId: string,
    paths: Array<{ path: string; role_hint?: string }>,
  ): Promise<MediaAsset[]> {
    return this.request<MediaAsset[]>(
      `/api/v1/projects/${encodeURIComponent(projectId)}/media/import`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ paths }),
      },
    );
  }

  deleteMedia(projectId: string, mediaId: string): Promise<Project> {
    return this.request<Project>(
      `/api/v1/projects/${encodeURIComponent(projectId)}/media/${encodeURIComponent(mediaId)}`,
      { method: "DELETE" },
    );
  }

  getMediaStreamUrl(projectId: string, mediaId: string): string {
    return this.appendTicket(
      `${this.baseUrl}/api/v1/projects/${encodeURIComponent(projectId)}/media/${encodeURIComponent(mediaId)}/stream`,
    );
  }

  getGfxSandboxUrl(sandboxId: string): string {
    return this.appendTicket(
      `${this.baseUrl}/api/v1/gfx/sandbox/${encodeURIComponent(sandboxId)}`,
    );
  }

  async registerGfxSandbox(html: string): Promise<string> {
    const key = hashGfxHtml(html);
    const cached = this.gfxSandboxCache.get(key);
    if (cached && cached.html === html) {
      this.gfxSandboxCache.delete(key);
      this.gfxSandboxCache.set(key, cached);
      return this.getGfxSandboxUrl(cached.id);
    }

    const res = await this.request<GfxSandboxResponse>("/api/v1/gfx/sandbox", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ html }),
    });

    if (this.gfxSandboxCache.size >= MAX_GFX_SANDBOX_CACHE_ENTRIES) {
      const oldestKey = this.gfxSandboxCache.keys().next().value;
      if (oldestKey !== undefined) {
        this.gfxSandboxCache.delete(oldestKey);
      }
    }
    this.gfxSandboxCache.set(key, { id: res.id, html });
    return this.getGfxSandboxUrl(res.id);
  }

  listBrands(): Promise<BrandPlaybook[]> {
    return this.request<BrandPlaybook[]>("/api/v1/brands");
  }

  createBrand(brand: BrandPlaybook): Promise<BrandPlaybook> {
    return this.request<BrandPlaybook>("/api/v1/brands", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(brand),
    });
  }

  saveBrand(brand: BrandPlaybook): Promise<BrandPlaybook> {
    return this.request<BrandPlaybook>(
      `/api/v1/brands/${encodeURIComponent(brand.id)}`,
      {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(brand),
      },
    );
  }

  uploadBrandMedia(
    brandId: string,
    files: File[],
    notes = "",
  ): Promise<BrandPlaybook> {
    const form = new FormData();
    if (notes) {
      form.append("notes", notes);
    }
    for (const file of files) {
      form.append("file", file, file.name);
    }
    return this.request<BrandPlaybook>(
      `/api/v1/brands/${encodeURIComponent(brandId)}/media`,
      {
        method: "POST",
        body: form,
      },
    );
  }

  updateBrandMedia(
    brandId: string,
    assetId: string,
    updates: { notes?: string; kind?: BrandReferenceKind },
  ): Promise<BrandPlaybook> {
    return this.request<BrandPlaybook>(
      `/api/v1/brands/${encodeURIComponent(brandId)}/media/${encodeURIComponent(assetId)}`,
      {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(updates),
      },
    );
  }

  deleteBrandMedia(brandId: string, assetId: string): Promise<BrandPlaybook> {
    return this.request<BrandPlaybook>(
      `/api/v1/brands/${encodeURIComponent(brandId)}/media/${encodeURIComponent(assetId)}`,
      {
        method: "DELETE",
      },
    );
  }

  getBrandMediaStreamUrl(brandId: string, assetId: string): string {
    return this.appendTicket(
      `${this.baseUrl}/api/v1/brands/${encodeURIComponent(brandId)}/media/${encodeURIComponent(assetId)}/stream`,
    );
  }

  getExportDownloadUrl(jobId: string): string {
    return this.appendTicket(
      `${this.baseUrl}/api/v1/exports/${encodeURIComponent(jobId)}/download`,
    );
  }

  getBrandLut(brandId: string): Promise<ParsedCubeLut> {
    return this.request<ParsedCubeLut>(
      `/api/v1/brands/${encodeURIComponent(brandId)}/lut`,
    );
  }

  startExport(projectId: string, req: ExportRequest): Promise<ExportJobStatus> {
    return this.request<ExportJobStatus>(
      `/api/v1/projects/${encodeURIComponent(projectId)}/export`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(req),
      },
    );
  }

  getExportJob(jobId: string): Promise<ExportJobStatus> {
    return this.request<ExportJobStatus>(
      `/api/v1/exports/${encodeURIComponent(jobId)}`,
    );
  }

  listAssets(): Promise<SavedCodeAsset[]> {
    return this.request<SavedCodeAsset[]>("/api/v1/assets");
  }

  saveAsset(asset: SavedCodeAsset): Promise<SavedCodeAsset> {
    return this.request<SavedCodeAsset>("/api/v1/assets", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(asset),
    });
  }

  exportAssetPack(ids?: string[]): Promise<CodeAssetPackManifest> {
    const query =
      ids && ids.length > 0
        ? `?ids=${encodeURIComponent(ids.join(","))}`
        : "";
    return this.request<CodeAssetPackManifest>(`/api/v1/assets/export${query}`);
  }

  importAssetPack(manifest: CodeAssetPackManifest): Promise<AssetPackImportResponse> {
    return this.request<AssetPackImportResponse>("/api/v1/assets/import", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(manifest),
    });
  }

  runProjectQa(projectId: string, autoFix = true): Promise<QaReport> {
    return this.request<QaReport>(
      `/api/v1/projects/${encodeURIComponent(projectId)}/qa`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ auto_fix: autoFix }),
      },
    );
  }

  getAgentStatus(): Promise<AgentStatusResponse> {
    return this.request<AgentStatusResponse>("/api/v1/agent/status");
  }

  setAgentCredentials(payload: {
    api_key?: string;
    default_model?: string;
    clear_default_model?: boolean;
  }): Promise<AgentStatusResponse> {
    return this.request<AgentStatusResponse>("/api/v1/agent/credentials", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    });
  }

  clearAgentCredentials(): Promise<AgentStatusResponse> {
    return this.request<AgentStatusResponse>("/api/v1/agent/credentials", {
      method: "DELETE",
    });
  }

  getRecommendedModels(): Promise<ModelAdvisorResponse> {
    return this.request<ModelAdvisorResponse>("/api/v1/agent/models");
  }

  listSkills(): Promise<SkillSummary[]> {
    return this.request<SkillSummary[]>("/api/v1/agent/skills");
  }

  generatePlan(req: GeneratePlanRequest): Promise<CreativePlan> {
    return this.request<CreativePlan>("/api/v1/agent/plan", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(req),
    });
  }

  getLogs(params?: {
    projectId?: string;
    level?: LogLevel;
    limit?: number;
  }): Promise<LogEntry[]> {
    const qs = new URLSearchParams();
    if (params?.projectId) qs.set("project_id", params.projectId);
    if (params?.level) qs.set("level", params.level);
    if (params?.limit) qs.set("limit", String(params.limit));
    const suffix = qs.toString() ? `?${qs.toString()}` : "";
    return this.request<LogEntry[]>(`/api/v1/logs${suffix}`);
  }

  clearLogs(projectId?: string): Promise<void> {
    const suffix = projectId ? `?project_id=${encodeURIComponent(projectId)}` : "";
    return this.request<void>(`/api/v1/logs${suffix}`, { method: "DELETE" });
  }

  startSwarmRun(projectId: string, req: SwarmRunRequest): Promise<SwarmRunStatus> {
    return this.request<SwarmRunStatus>(
      `/api/v1/projects/${encodeURIComponent(projectId)}/swarm/run`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(req),
      },
    );
  }

  getSwarmRun(projectId: string, runId: string): Promise<SwarmRunStatus> {
    return this.request<SwarmRunStatus>(
      `/api/v1/projects/${encodeURIComponent(projectId)}/swarm/${encodeURIComponent(runId)}`,
    );
  }

  cancelSwarmRun(projectId: string, runId: string): Promise<void> {
    return this.request<void>(
      `/api/v1/projects/${encodeURIComponent(projectId)}/swarm/${encodeURIComponent(runId)}/cancel`,
      { method: "POST" },
    );
  }

  transcribeMedia(
    projectId: string,
    mediaId: string,
    language?: string,
  ): Promise<{ media_id: string; word_count: number; words: TranscriptWord[] }> {
    return this.request<{ media_id: string; word_count: number; words: TranscriptWord[] }>(
      `/api/v1/projects/${encodeURIComponent(projectId)}/media/${encodeURIComponent(mediaId)}/transcribe`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ language }),
      },
    );
  }

  subscribeEvents(
    projectId: string,
    onEvent: (envelope: EngineEventEnvelope) => void,
    onStatusChange?: (connected: boolean) => void,
    onReconnect?: () => void,
  ): () => void {
    let closed = false;
    let ws: WebSocket | null = null;
    let timer: ReturnType<typeof setTimeout> | null = null;
    let backoffMs = 1000;
    let hasConnectedOnce = false;

    const originFallback =
      typeof window !== "undefined" && window.location?.origin
        ? window.location.origin
        : "http://127.0.0.1:41731";
    const resolvedBase = this.baseUrl
      ? new URL(this.baseUrl, originFallback).toString().replace(/\/+$/, "")
      : originFallback.replace(/\/+$/, "");
    const wsBase = resolvedBase.replace(/^http/i, "ws");
    const url = `${wsBase}/api/v1/ws?project_id=${encodeURIComponent(projectId)}`;

    const scheduleReconnect = () => {
      if (closed) return;
      const jitter = Math.floor(Math.random() * 350);
      const delay = backoffMs + jitter;
      backoffMs = Math.min(15_000, backoffMs * 2);
      timer = setTimeout(() => void connect(), delay);
    };

    const connect = async () => {
      if (closed) return;
      if ((this.authToken || this.hasSession) && Date.now() >= this.ticketExpiresAt - 15_000) {
        await this.establishSession(true).catch(() => {});
      }
      if (closed) return;
      const protocols = this.mediaTicket
        ? ["solaris.v1", `solaris.ticket.${this.mediaTicket}`]
        : undefined;
      try {
        ws = protocols ? new WebSocket(url, protocols) : new WebSocket(url);
      } catch {
        onStatusChange?.(false);
        scheduleReconnect();
        return;
      }
      ws.onopen = () => {
        backoffMs = 1000;
        onStatusChange?.(true);
        if (hasConnectedOnce) {
          onReconnect?.();
        }
        hasConnectedOnce = true;
      };
      ws.onmessage = (msg) => {
        try {
          const parsed = JSON.parse(String(msg.data)) as EngineEventEnvelope;
          onEvent(parsed);
        } catch {
          // Ignore malformed frame
        }
      };
      ws.onclose = () => {
        onStatusChange?.(false);
        scheduleReconnect();
      };
      ws.onerror = () => {
        ws?.close();
      };
    };

    void connect();

    return () => {
      closed = true;
      if (timer) clearTimeout(timer);
      ws?.close();
    };
  }
}
