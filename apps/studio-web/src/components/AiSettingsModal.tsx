import { useEffect, useState, type FormEvent } from "react";
import {
  Check,
  Copy,
  Cpu,
  KeyRound,
  RefreshCw,
  ShieldCheck,
  Sparkles,
  Terminal,
  Trash2,
  X,
} from "lucide-react";
import type {
  AgentStatusResponse,
  ModelAdvisorResponse,
  ModelCandidate,
  SolarisEngineClient,
} from "@solaris/engine-client";
import {
  deleteDesktopOpenRouterKey,
  isDesktopTauri,
  saveDesktopOpenRouterKey,
} from "../lib/engineBridge";
import { SolarSelect } from "./ui/SolarSelect";

interface AiSettingsModalProps {
  open: boolean;
  client: SolarisEngineClient;
  agentStatus: AgentStatusResponse | null;
  modelAdvisor: ModelAdvisorResponse | null;
  selectedModelOverride: string;
  onSelectModelOverride: (modelId: string) => void;
  onAgentStatusChange: (status: AgentStatusResponse) => void;
  onRefreshModels: () => Promise<void>;
  onClose: () => void;
}

export function isOmniModalModel(candidate: ModelCandidate): boolean {
  if (candidate.match_tier) {
    return candidate.match_tier === "best_match";
  }
  const inputs = candidate.input_modalities ?? [];
  return (
    candidate.supports_tools &&
    inputs.includes("video") &&
    inputs.includes("audio") &&
    inputs.includes("image") &&
    inputs.includes("text")
  );
}

export function isVisionFallbackModel(candidate: ModelCandidate): boolean {
  if (candidate.match_tier) {
    return candidate.match_tier === "smart_fallback";
  }
  const inputs = candidate.input_modalities ?? [];
  return (
    candidate.supports_tools &&
    inputs.includes("image") &&
    inputs.includes("text") &&
    !isOmniModalModel(candidate)
  );
}

export function AiSettingsModal({
  open,
  client,
  agentStatus,
  modelAdvisor,
  selectedModelOverride,
  onSelectModelOverride,
  onAgentStatusChange,
  onRefreshModels,
  onClose,
}: AiSettingsModalProps) {
  const [apiKeyInput, setApiKeyInput] = useState("");
  const [savingKey, setSavingKey] = useState(false);
  const [savingModel, setSavingModel] = useState(false);
  const [refreshingModels, setRefreshingModels] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [successMsg, setSuccessMsg] = useState<string | null>(null);
  const [copiedId, setCopiedId] = useState<string | null>(null);

  const desktopKeychain = isDesktopTauri();
  const mcpEndpoint = `${client.getBaseUrl()}${agentStatus?.mcp_http_path ?? "/mcp"}`;

  useEffect(() => {
    if (!open) return;
    setErrorMsg(null);
    setSuccessMsg(null);
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  if (!open) return null;

  const copyText = (id: string, text: string) => {
    const fallbackCopy = () => {
      try {
        const area = document.createElement("textarea");
        area.value = text;
        area.style.position = "fixed";
        area.style.opacity = "0";
        document.body.appendChild(area);
        area.select();
        document.execCommand("copy");
        area.remove();
        setCopiedId(id);
        setTimeout(() => setCopiedId((prev) => (prev === id ? null : prev)), 1600);
      } catch {
        setErrorMsg("Clipboard access denied. Copy the command manually.");
      }
    };
    if (navigator.clipboard?.writeText) {
      navigator.clipboard
        .writeText(text)
        .then(() => {
          setCopiedId(id);
          setTimeout(() => setCopiedId((prev) => (prev === id ? null : prev)), 1600);
        })
        .catch(fallbackCopy);
    } else {
      fallbackCopy();
    }
  };

  const handleSaveKey = async (e: FormEvent) => {
    e.preventDefault();
    const trimmed = apiKeyInput.trim();
    if (!trimmed || savingKey) return;
    setSavingKey(true);
    setErrorMsg(null);
    setSuccessMsg(null);
    try {
      const updated = await client.setAgentCredentials({ api_key: trimmed });
      if (desktopKeychain) {
        await saveDesktopOpenRouterKey(trimmed);
      }
      onAgentStatusChange(updated);
      setApiKeyInput("");
      setSuccessMsg(
        desktopKeychain
          ? "OpenRouter API key saved to OS Keychain and activated."
          : "OpenRouter API key saved locally (owner-only disk file, never in browser storage) and activated.",
      );
      void onRefreshModels().catch(() => {});
    } catch (err) {
      setErrorMsg(err instanceof Error ? err.message : "Failed to save API key");
    } finally {
      setSavingKey(false);
    }
  };

  const handleClearKey = async () => {
    if (savingKey) return;
    setSavingKey(true);
    setErrorMsg(null);
    setSuccessMsg(null);
    try {
      const updated = await client.clearAgentCredentials();
      if (desktopKeychain) {
        await deleteDesktopOpenRouterKey();
      }
      onAgentStatusChange(updated);
      setSuccessMsg("OpenRouter API key cleared from memory and local storage.");
    } catch (err) {
      setErrorMsg(err instanceof Error ? err.message : "Failed to clear API key");
    } finally {
      setSavingKey(false);
    }
  };

  const handlePickModel = async (modelId: string) => {
    setSavingModel(true);
    setErrorMsg(null);
    try {
      const updated =
        modelId === ""
          ? await client.setAgentCredentials({ clear_default_model: true })
          : await client.setAgentCredentials({ default_model: modelId });
      onAgentStatusChange(updated);
      onSelectModelOverride(modelId);
      setSuccessMsg(
        modelId
          ? `Active Swarm model set to ${modelId}.`
          : "Swarm model reset to Model Advisor automatic selection.",
      );
    } catch (err) {
      setErrorMsg(err instanceof Error ? err.message : "Failed to update model");
    } finally {
      setSavingModel(false);
    }
  };

  const directorCandidates = modelAdvisor?.director.candidates ?? [];
  const omniCandidates = directorCandidates.filter(isOmniModalModel).slice(0, 6);
  const programmaticCandidates = directorCandidates
    .filter((cand) => !isOmniModalModel(cand))
    .slice(0, 6);

  const renderModelCard = (cand: ModelCandidate) => {
    const isAutoPrimary = !selectedModelOverride && modelAdvisor?.director.primary === cand.id;
    const active = selectedModelOverride === cand.id || isAutoPrimary;
    return (
      <button
        key={cand.id}
        type="button"
        disabled={savingModel}
        onClick={() => void handlePickModel(cand.id)}
        className={`flex w-full items-center justify-between gap-2 rounded-xl border px-3 py-2 text-left transition-colors motion-reduce:transition-none ${
          active
            ? "border-laranja-criacao/65 bg-accent"
            : "border-areia/10 bg-carbono/65 hover:border-areia/25"
        }`}
      >
        <div className="min-w-0">
          <div className="flex items-center gap-1.5">
            <span className="truncate font-medium text-areia">{cand.id}</span>
            {isAutoPrimary && (
              <span className="shrink-0 rounded-full border border-amarelo-ideias/40 bg-amarelo-ideias/10 px-1.5 py-0.5 text-[10px] font-medium text-amarelo-ideias">
                Auto
              </span>
            )}
          </div>
          <div className="mt-0.5 truncate text-[10px] text-areia/55">
            {cand.input_modalities.join(", ") || "text"} ·{" "}
            <span className="tabular-nums">
              {cand.context_length > 0 ? `${Math.round(cand.context_length / 1000)}k` : "n/a"}
            </span>
          </div>
        </div>
        <span className="tabular-nums shrink-0 text-[10px] text-amarelo-ideias">
          {typeof cand.prompt_usd_per_m === "number"
            ? `$${cand.prompt_usd_per_m.toFixed(2)}/M`
            : "n/a"}
        </span>
      </button>
    );
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="ai-settings-title"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
      className="fixed inset-0 z-50 grid place-items-center bg-void/80 p-4 backdrop-blur-md"
    >
      <div className="flex max-h-[88dvh] w-full max-w-3xl flex-col overflow-hidden rounded-2xl border border-areia/14 bg-carbono shadow-2xl">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-areia/10 px-5 py-3.5">
          <div className="flex items-center gap-2.5">
            <Cpu className="h-4 w-4 text-amarelo-ideias" />
            <h2 id="ai-settings-title" className="text-sm font-semibold text-areia">
              AI Director, OpenRouter BYOK &amp; MCP Mode
            </h2>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="grid h-8 w-8 place-items-center rounded-full border border-areia/12 text-areia/75 hover:text-areia"
            aria-label="Close AI settings"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="flex-1 space-y-5 overflow-y-auto p-5 text-xs">
          {errorMsg && (
            <div className="rounded-xl border border-vermelho-foco/45 bg-vermelho-foco/15 px-3 py-2 text-areia">
              {errorMsg}
            </div>
          )}
          {successMsg && (
            <div className="rounded-xl border border-amarelo-ideias/35 bg-amarelo-ideias/10 px-3 py-2 text-areia">
              {successMsg}
            </div>
          )}

          {/* Section 1: BYOK OpenRouter Key */}
          <section className="space-y-3 rounded-2xl border border-areia/12 bg-void/60 p-4">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="flex items-center gap-2">
                <KeyRound className="h-4 w-4 text-laranja-criacao" />
                <span className="text-xs font-semibold text-areia">
                  OpenRouter API Key (BYOK)
                </span>
              </div>
              <span
                className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-[10px] font-medium ${
                  agentStatus?.openrouter_configured
                    ? "border-amarelo-ideias/45 bg-amarelo-ideias/15 text-amarelo-ideias"
                    : "border-areia/15 bg-grafite/70 text-areia/65"
                }`}
              >
                <ShieldCheck className="h-3 w-3" />
                {agentStatus?.openrouter_configured
                  ? desktopKeychain
                    ? "Connected · OS Keychain"
                    : "Connected · Local Vault"
                  : "Not Configured (Local Fallback)"}
              </span>
            </div>

            <p className="text-[11px] leading-relaxed text-areia/65">
              Bring your own OpenRouter key so Solaris can route multimodal video/audio Director
              planning, parallel specialist tool calls, and Stage-3 Vision QA. Keys are never
              stored in <code className="font-sans tabular-nums text-areia/85">localStorage</code> or echoed back to the browser.
            </p>

            <form onSubmit={(e) => void handleSaveKey(e)} className="flex flex-wrap items-center gap-2">
              <input
                type="password"
                autoComplete="off"
                value={apiKeyInput}
                onChange={(e) => setApiKeyInput(e.target.value)}
                placeholder="sk-or-v1-..."
                className="h-9 min-w-[220px] flex-1 rounded-xl border border-areia/14 bg-carbono px-3 text-xs text-areia placeholder:text-areia/35 outline-none focus:border-laranja-criacao"
              />
              <button
                type="submit"
                disabled={savingKey || !apiKeyInput.trim()}
                className="btn-solar-outline h-9 px-4 text-xs font-medium text-areia disabled:opacity-45"
              >
                {savingKey ? "Saving..." : "Save Key"}
              </button>
              {agentStatus?.openrouter_configured && (
                <button
                  type="button"
                  disabled={savingKey}
                  onClick={() => void handleClearKey()}
                  className="flex h-9 items-center gap-1 rounded-full border border-vermelho-foco/45 px-3 text-xs text-areia/85 hover:bg-vermelho-foco/20 disabled:opacity-45"
                >
                  <Trash2 className="h-3.5 w-3.5 text-vermelho-foco" />
                  <span>Clear</span>
                </button>
              )}
            </form>
          </section>

          {/* Section 2: Model Advisor */}
          <section className="space-y-3 rounded-2xl border border-areia/12 bg-void/60 p-4">
            <div className="flex items-center justify-between gap-2">
              <div>
                <div className="flex items-center gap-1.5 font-semibold text-areia">
                  <Sparkles className="h-3.5 w-3.5 text-amarelo-ideias" />
                  <span>OpenRouter Model Advisor</span>
                </div>
                <p className="mt-0.5 text-[10px] text-areia/55">
                  Curated top models: native Omni-Modal (video + audio + tools) and fast, low-cost Programmatic editors.
                </p>
              </div>
              <button
                type="button"
                disabled={refreshingModels}
                onClick={async () => {
                  setRefreshingModels(true);
                  try {
                    await onRefreshModels();
                  } finally {
                    setRefreshingModels(false);
                  }
                }}
                className="flex items-center gap-1 rounded-full border border-areia/14 bg-carbono px-2.5 py-1 text-[10px] text-areia/80 hover:border-amarelo-ideias/50 disabled:opacity-45"
              >
                <RefreshCw
                  className={`h-3 w-3 text-amarelo-ideias ${
                    refreshingModels ? "animate-spin motion-reduce:animate-none" : ""
                  }`}
                />
                <span>Refresh</span>
              </button>
            </div>

            <div className="flex items-center justify-between gap-2">
              <span className="text-[10px] font-medium uppercase tracking-[0.14em] text-areia/55">
                Active Swarm Model
              </span>
              {selectedModelOverride && (
                <button
                  type="button"
                  disabled={savingModel}
                  onClick={() => void handlePickModel("")}
                  className="rounded-full border border-areia/14 bg-carbono px-2.5 py-0.5 text-[10px] text-amarelo-ideias hover:border-amarelo-ideias/45 disabled:opacity-45"
                >
                  Reset to Auto
                </button>
              )}
            </div>

            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <div className="space-y-2">
                <div className="rounded-xl border border-amarelo-ideias/30 bg-amarelo-ideias/10 p-2.5">
                  <span className="inline-block rounded-full bg-amarelo-ideias px-2 py-0.5 text-[10px] font-semibold text-void">
                    BEST MATCH · OMNI-MODAL
                  </span>
                  <p className="mt-1.5 text-[11px] leading-relaxed text-areia/85">
                    Video + Audio + Image + Text + Tools. Sees raw footage, hears pacing,
                    and inspects QA frames natively.
                  </p>
                </div>
                {omniCandidates.length > 0 && (
                  <div className="space-y-1.5">
                    {omniCandidates.map(renderModelCard)}
                  </div>
                )}
              </div>

              <div className="space-y-2">
                <div className="rounded-xl border border-areia/12 bg-carbono/80 p-2.5">
                  <span className="inline-block rounded-full border border-laranja-criacao/45 bg-laranja-criacao/15 px-2 py-0.5 text-[10px] font-semibold text-areia">
                    PROGRAMMATIC · FAST &amp; CHEAP
                  </span>
                  <p className="mt-1.5 text-[11px] leading-relaxed text-areia/75">
                    Vision + Text + Tools. Edits timelines &amp; GFX programmatically using
                    keyframes, transcripts, and beat JSON at low cost.
                  </p>
                </div>
                {programmaticCandidates.length > 0 && (
                  <div className="space-y-1.5">
                    {programmaticCandidates.map(renderModelCard)}
                  </div>
                )}
              </div>
            </div>

            {directorCandidates.length > 12 && (
              <SolarSelect
                variant="field"
                wrapperClassName="w-full"
                disabled={savingModel}
                aria-label="Browse all OpenRouter models"
                value={selectedModelOverride}
                onChange={(e) => void handlePickModel(e.target.value)}
                className="bg-carbono"
              >
                <option value="" className="bg-carbono">
                  Auto Recommended ({modelAdvisor?.director.primary ?? "google/gemini-4-hydrogen"})
                </option>
                {directorCandidates.map((cand) => {
                  const badge = isOmniModalModel(cand)
                    ? "[Omni-Modal]"
                    : isVisionFallbackModel(cand)
                      ? "[Programmatic]"
                      : "[Tools]";
                  const price =
                    typeof cand.prompt_usd_per_m === "number"
                      ? ` · $${cand.prompt_usd_per_m.toFixed(2)}/M`
                      : "";
                  return (
                    <option key={cand.id} value={cand.id} className="bg-carbono">
                      {cand.id} {badge}
                      {price}
                    </option>
                  );
                })}
              </SolarSelect>
            )}
          </section>

          {/* Section 3: External MCP Agent Mode */}
          <section className="space-y-2.5 rounded-2xl border border-areia/12 bg-void/60 p-4">
            <div className="flex items-center gap-2 font-semibold text-areia">
              <Terminal className="h-4 w-4 text-laranja-criacao" />
              <span>External MCP Agent Mode (Codex · Antigravity · Claude Code)</span>
            </div>
            <p className="text-[11px] text-areia/65">
              Connect any external MCP client directly to Solaris&apos;s Streamable HTTP endpoint or
              install all 6 bundled directing skills locally:
            </p>
            <div className="space-y-1.5">
              {[
                { id: "mcp-url", label: "Streamable HTTP Endpoint", value: mcpEndpoint },
                {
                  id: "skills-agy",
                  label: "Install Skills (Antigravity)",
                  value: "solaris-mcp install-skills --target antigravity",
                },
                {
                  id: "skills-codex",
                  label: "Install Skills (Codex / Claude)",
                  value: "solaris-mcp install-skills --target codex",
                },
              ].map((row) => (
                <div
                  key={row.id}
                  className="flex items-center justify-between gap-2 rounded-xl border border-areia/10 bg-carbono px-3 py-2"
                >
                  <div className="min-w-0">
                    <div className="text-[10px] uppercase tracking-[0.12em] text-areia/50">
                      {row.label}
                    </div>
                    <code className="block truncate font-sans tabular-nums text-[11px] text-areia/90">
                      {row.value}
                    </code>
                  </div>
                  <button
                    type="button"
                    onClick={() => copyText(row.id, row.value)}
                    className="flex shrink-0 items-center gap-1 rounded-full border border-areia/14 bg-void/70 px-2.5 py-1 text-[10px] text-areia/80 hover:border-amarelo-ideias/50"
                  >
                    {copiedId === row.id ? (
                      <Check className="h-3 w-3 text-amarelo-ideias" />
                    ) : (
                      <Copy className="h-3 w-3 text-laranja-criacao" />
                    )}
                    <span>{copiedId === row.id ? "Copied" : "Copy"}</span>
                  </button>
                </div>
              ))}
            </div>
          </section>
        </div>

        <div className="flex justify-end border-t border-areia/10 px-5 py-3">
          <button
            type="button"
            onClick={onClose}
            className="rounded-full border border-areia/15 px-4 py-1.5 text-xs text-areia/85 hover:bg-accent"
          >
            Done
          </button>
        </div>
      </div>
    </div>
  );
}
