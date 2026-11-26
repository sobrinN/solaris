import { useEffect, useRef, useState } from "react";
import {
  FileText,
  Film,
  Image as ImageIcon,
  Music,
  Palette,
  Plus,
  Save,
  Sparkles,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import type {
  BrandPlaybook,
  BrandReferenceAsset,
  BrandReferenceKind,
  SolarisEngineClient,
} from "@solaris/engine-client";
import { SolarSelect } from "./ui/SolarSelect";

interface BrandPlaybookDrawerProps {
  open: boolean;
  client: SolarisEngineClient;
  brands: BrandPlaybook[];
  selectedBrandId: string;
  onSelectBrand: (id: string) => void;
  onClose: () => void;
  onSave: (updated: BrandPlaybook) => Promise<void>;
  onCreateBrand: (created: BrandPlaybook) => Promise<void>;
  onBrandRefreshed: (updated: BrandPlaybook) => void;
}

const KIND_LABELS: Record<BrandReferenceKind, string> = {
  reference_video: "VIDEO REF",
  style_frame: "STYLE FRAME",
  logo: "LOGO",
  audio_identity: "AUDIO",
  lut_cube: "LUT .CUBE",
  font: "FONT",
  document: "DOC",
};

export function BrandPlaybookDrawer({
  open,
  client,
  brands,
  selectedBrandId,
  onSelectBrand,
  onClose,
  onSave,
  onCreateBrand,
  onBrandRefreshed,
}: BrandPlaybookDrawerProps) {
  const brand = brands.find((b) => b.id === selectedBrandId) ?? brands[0] ?? null;
  const brandRef = useRef<BrandPlaybook | null>(brand);
  brandRef.current = brand;
  const [draft, setDraft] = useState<BrandPlaybook | null>(brand);
  const [saving, setSaving] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (open) {
      setDraft(brandRef.current);
      setErrorMsg(null);
    }
  }, [open, brand?.id]);

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  if (!open || !draft) return null;

  const mergeServerMediaFields = (serverBrand: BrandPlaybook) => {
    setDraft((prev) =>
      prev
        ? {
            ...prev,
            reference_media: serverBrand.reference_media,
            default_lut: serverBrand.default_lut,
            updated_at: serverBrand.updated_at,
          }
        : serverBrand,
    );
    onBrandRefreshed(serverBrand);
  };

  const handleSave = async () => {
    setSaving(true);
    setErrorMsg(null);
    try {
      await onSave(draft);
      onClose();
    } catch (err) {
      setErrorMsg(err instanceof Error ? err.message : "Failed to save brand playbook");
    } finally {
      setSaving(false);
    }
  };

  const handleCreateNew = async () => {
    setErrorMsg(null);
    try {
      const id = `brand-${Date.now()}`;
      const next: BrandPlaybook = {
        ...draft,
        id,
        name: `New Brand (${brands.length + 1})`,
        reference_media: [],
        default_lut: null,
        updated_at: new Date().toISOString(),
      };
      await onCreateBrand(next);
      onSelectBrand(id);
    } catch (err) {
      setErrorMsg(err instanceof Error ? err.message : "Failed to create brand");
    }
  };

  const handleUploadFiles = async (incoming: FileList | null) => {
    if (!incoming || incoming.length === 0) return;
    setUploading(true);
    setErrorMsg(null);
    try {
      const updated = await client.uploadBrandMedia(
        draft.id,
        Array.from(incoming),
      );
      mergeServerMediaFields(updated);
    } catch (err) {
      setErrorMsg(err instanceof Error ? err.message : "Failed to upload reference media");
    } finally {
      setUploading(false);
    }
  };

  const handleDeleteRef = async (assetId: string) => {
    setErrorMsg(null);
    try {
      const updated = await client.deleteBrandMedia(draft.id, assetId);
      mergeServerMediaFields(updated);
    } catch (err) {
      setErrorMsg(err instanceof Error ? err.message : "Failed to delete reference asset");
    }
  };

  const handleNoteBlur = async (item: BrandReferenceAsset, notes: string) => {
    if (item.notes === notes) return;
    setErrorMsg(null);
    try {
      const updated = await client.updateBrandMedia(draft.id, item.id, { notes });
      mergeServerMediaFields(updated);
    } catch (err) {
      setErrorMsg(err instanceof Error ? err.message : "Failed to update note");
    }
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="brand-drawer-title"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
      className="fixed inset-0 z-50 flex justify-end bg-void/70 backdrop-blur-sm"
    >
      <div className="flex h-full w-full max-w-lg flex-col border-l border-areia/12 bg-carbono shadow-2xl">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-areia/10 px-5 py-4">
          <div className="flex items-center gap-2.5">
            <Palette className="h-4 w-4 text-laranja-criacao" />
            <h2
              id="brand-drawer-title"
              className="text-sm font-semibold tracking-[-0.015em] text-areia"
            >
              Brand Playbook &amp; Media Kit
            </h2>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="grid h-9 w-9 place-items-center rounded-full border border-areia/12 text-areia/75 hover:border-areia/30 hover:text-areia"
            aria-label="Close Brand Playbook"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* Playbook Selector */}
        <div className="flex items-center gap-2 border-b border-areia/10 bg-void/40 px-5 py-2.5">
          <SolarSelect
            wrapperClassName="min-w-0 flex-1"
            value={draft.id}
            onChange={(e) => onSelectBrand(e.target.value)}
            className="h-8 w-full bg-grafite/80"
            aria-label="Select Brand Playbook"
          >
            {brands.map((b) => (
              <option key={b.id} value={b.id} className="bg-carbono">
                {b.name}
              </option>
            ))}
          </SolarSelect>
          <button
            type="button"
            onClick={() => void handleCreateNew()}
            className="flex h-8 items-center gap-1 rounded-full border border-areia/14 bg-void/70 px-3 text-[11px] text-areia/85 hover:border-amarelo-ideias/50 hover:bg-accent"
          >
            <Plus className="h-3.5 w-3.5 text-amarelo-ideias" />
            <span>New</span>
          </button>
        </div>

        <div className="flex-1 space-y-4 overflow-y-auto p-5 text-xs">
          {errorMsg && (
            <div className="rounded-xl border border-vermelho-foco/40 bg-vermelho-foco/15 p-2.5 text-[11px] text-areia">
              {errorMsg}
            </div>
          )}

          <div>
            <label className="mb-1.5 block text-[11px] font-medium uppercase tracking-[0.14em] text-areia/60">
              Playbook Name
            </label>
            <input
              value={draft.name}
              onChange={(e) => setDraft({ ...draft, name: e.target.value })}
              className="w-full rounded-xl border border-areia/12 bg-void/70 px-3 py-2 text-sm text-areia outline-none focus:border-laranja-criacao"
            />
          </div>

          {/* Brand Reference Media & Visual Identity */}
          <div className="space-y-2.5 rounded-2xl border border-areia/12 bg-void/50 p-3.5">
            <div className="flex items-center justify-between">
              <div>
                <span className="block text-[11px] font-medium uppercase tracking-[0.14em] text-amarelo-ideias">
                  Reference Media &amp; Brand Kit
                </span>
                <span className="text-[10px] text-areia/55">
                  Attach reference reels, style frames, logos, .cube LUTs, fonts, or audio beds so the LLM learns your exact look &amp; pacing.
                </span>
              </div>
            </div>

            <input
              ref={fileInputRef}
              type="file"
              multiple
              accept="video/*,image/*,audio/*,.cube,.woff2,.woff,.ttf,.otf,.pdf,.md,.txt"
              onChange={(e) => {
                void handleUploadFiles(e.target.files);
                e.currentTarget.value = "";
              }}
              className="hidden"
            />

            <button
              type="button"
              onDragOver={(e) => e.preventDefault()}
              onDrop={(e) => {
                e.preventDefault();
                void handleUploadFiles(e.dataTransfer.files);
              }}
              onClick={() => fileInputRef.current?.click()}
              className="flex w-full cursor-pointer items-center justify-center gap-2 rounded-xl border border-dashed border-areia/20 bg-carbono/60 px-3 py-3 text-[11px] text-areia/75 transition-colors hover:border-laranja-criacao/60 hover:bg-accent motion-reduce:transition-none"
            >
              <Upload className="h-3.5 w-3.5 text-laranja-criacao" />
              <span>
                {uploading
                  ? "Uploading & analyzing..."
                  : "Drop or click to attach style frames, videos, logos, LUTs (.cube), fonts"}
              </span>
            </button>

            {(draft.reference_media?.length ?? 0) > 0 && (
              <div className="space-y-2 pt-1">
                {draft.reference_media.map((item) => {
                  const isImage =
                    item.kind === "style_frame" || item.kind === "logo";
                  return (
                    <div
                      key={item.id}
                      className="flex items-start gap-2.5 rounded-xl border border-areia/10 bg-carbono/80 p-2.5"
                    >
                      <div className="grid h-12 w-12 shrink-0 place-items-center overflow-hidden rounded-lg border border-areia/10 bg-void">
                        {isImage ? (
                          <img
                            src={client.getBrandMediaStreamUrl(draft.id, item.id)}
                            alt={item.name}
                            className="h-full w-full object-cover"
                          />
                        ) : (
                          <RefKindIcon kind={item.kind} />
                        )}
                      </div>

                      <div className="min-w-0 flex-1 space-y-1">
                        <div className="flex items-center justify-between gap-2">
                          <span className="truncate text-[11px] font-medium text-areia">
                            {item.name}
                          </span>
                          <span className="shrink-0 rounded bg-grafite px-1.5 py-0.5 text-[10px] font-semibold text-amarelo-ideias">
                            {KIND_LABELS[item.kind]}
                          </span>
                        </div>

                        {item.intelligence?.analyzed && (
                          <div className="tabular-nums text-[10px] text-areia/50">
                            {item.intelligence.duration_sec > 0 &&
                              `${item.intelligence.duration_sec.toFixed(1)}s`}
                            {item.intelligence.width &&
                              ` · ${item.intelligence.width}×${item.intelligence.height}`}
                            {typeof item.intelligence.integrated_lufs === "number" &&
                              ` · ${item.intelligence.integrated_lufs.toFixed(1)} LUFS`}
                          </div>
                        )}

                        <input
                          value={item.notes}
                          placeholder="Tell the LLM what to learn from this asset (e.g. 'use this caption bounce & warm highlights')..."
                          onChange={(e) => {
                            const val = e.target.value;
                            setDraft((prev) =>
                              prev
                                ? {
                                    ...prev,
                                    reference_media: prev.reference_media.map((r) =>
                                      r.id === item.id ? { ...r, notes: val } : r,
                                    ),
                                  }
                                : prev,
                            );
                          }}
                          onBlur={(e) => {
                            const savedItem = brand?.reference_media.find(
                              (r) => r.id === item.id,
                            );
                            if (savedItem && savedItem.notes !== e.target.value) {
                              void handleNoteBlur(savedItem, e.target.value);
                            }
                          }}
                          className="w-full rounded-lg border border-areia/10 bg-void/70 px-2 py-1 text-[10px] text-areia/85 placeholder:text-areia/35 outline-none focus:border-laranja-criacao"
                        />
                      </div>

                      <button
                        type="button"
                        onClick={() => void handleDeleteRef(item.id)}
                        className="rounded-lg p-1 text-areia/45 hover:bg-vermelho-foco/20 hover:text-areia"
                        aria-label={`Delete ${item.name}`}
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </button>
                    </div>
                  );
                })}
              </div>
            )}
          </div>

          <div>
            <label className="mb-1.5 block text-[11px] font-medium uppercase tracking-[0.14em] text-areia/60">
              BRAND.md Directing Rules
            </label>
            <textarea
              rows={6}
              value={draft.brand_md}
              onChange={(e) => setDraft({ ...draft, brand_md: e.target.value })}
              className="w-full rounded-xl border border-areia/12 bg-void/70 p-3 text-xs leading-relaxed text-areia outline-none focus:border-laranja-criacao"
            />
          </div>

          <div>
            <label className="mb-1.5 block text-[11px] font-medium uppercase tracking-[0.14em] text-areia/60">
              Pacing Cadence
            </label>
            <input
              value={draft.pacing_rules}
              onChange={(e) => setDraft({ ...draft, pacing_rules: e.target.value })}
              className="w-full rounded-xl border border-areia/12 bg-void/70 px-3 py-2 text-xs text-areia outline-none focus:border-laranja-criacao"
            />
          </div>

          <div>
            <span className="mb-2 block text-[11px] font-medium uppercase tracking-[0.14em] text-areia/60">
              Injected CSS Design Tokens
            </span>
            <div className="grid grid-cols-2 gap-2.5">
              {(
                [
                  ["bg", "Background"],
                  ["surface", "Surface"],
                  ["ink", "Ink"],
                  ["accent", "Primary Accent"],
                  ["accent_secondary", "Rim Highlight"],
                ] as const
              ).map(([key, label]) => (
                <label
                  key={key}
                  className="flex items-center justify-between rounded-xl border border-areia/10 bg-void/60 px-3 py-2"
                >
                  <span className="text-[11px] text-areia/75">{label}</span>
                  <div className="flex items-center gap-2">
                    <span className="tabular-nums text-[10px] text-areia/55">
                      {draft.tokens[key]}
                    </span>
                    <input
                      type="color"
                      value={draft.tokens[key]}
                      onChange={(e) =>
                        setDraft({
                          ...draft,
                          tokens: { ...draft.tokens, [key]: e.target.value },
                        })
                      }
                      className="h-5 w-5 cursor-pointer rounded border-0 bg-transparent p-0"
                    />
                  </div>
                </label>
              ))}
            </div>

            <label className="mt-3 flex cursor-pointer items-center justify-between gap-3 rounded-xl border border-areia/10 bg-void/60 px-3 py-2.5">
              <div className="min-w-0">
                <span className="block text-[11px] font-medium text-areia">
                  Lock Brand Palette
                </span>
                <span className="block text-[10px] text-areia/55">
                  Enforce these brand colors even when a prompt requests different hex colors.
                </span>
              </div>
              <input
                type="checkbox"
                checked={Boolean(draft.lock_brand_palette)}
                onChange={(e) =>
                  setDraft({ ...draft, lock_brand_palette: e.target.checked })
                }
                className="h-4 w-4 shrink-0 cursor-pointer accent-laranja-criacao"
              />
            </label>

            {/* Brand Typography Tokens (Display & Body) */}
            <div className="mt-3 grid grid-cols-1 gap-2.5 sm:grid-cols-2">
              <label className="block space-y-1">
                <span className="block text-[10px] uppercase tracking-[0.12em] text-areia/60">
                  Display Font
                </span>
                <input
                  value={draft.tokens.font_display}
                  onChange={(e) =>
                    setDraft({
                      ...draft,
                      tokens: { ...draft.tokens, font_display: e.target.value },
                    })
                  }
                  placeholder="Sora"
                  className="w-full rounded-xl border border-areia/12 bg-void/70 px-3 py-1.5 text-xs text-areia outline-none focus:border-laranja-criacao"
                />
              </label>
              <label className="block space-y-1">
                <span className="block text-[10px] uppercase tracking-[0.12em] text-areia/60">
                  Body Font
                </span>
                <input
                  value={draft.tokens.font_body}
                  onChange={(e) =>
                    setDraft({
                      ...draft,
                      tokens: { ...draft.tokens, font_body: e.target.value },
                    })
                  }
                  placeholder="Sora"
                  className="w-full rounded-xl border border-areia/12 bg-void/70 px-3 py-1.5 text-xs text-areia outline-none focus:border-laranja-criacao"
                />
              </label>
            </div>

            {(() => {
              const uploadedFonts = (draft.reference_media ?? [])
                .filter((r) => r.kind === "font")
                .map((r) => ({
                  id: r.id,
                  family: r.name
                    .replace(/\.[^.]+$/, "")
                    .replace(/[^a-zA-Z0-9 _-]/g, "")
                    .trim(),
                }))
                .filter((f) => Boolean(f.family));
              if (uploadedFonts.length === 0) return null;
              return (
                <div className="mt-2 space-y-1.5">
                  <span className="block text-[10px] text-areia/50">
                    Uploaded brand fonts (click to apply):
                  </span>
                  <div className="flex flex-wrap gap-1.5">
                    {uploadedFonts.map((f) => (
                      <div
                        key={f.id}
                        className="inline-flex items-center gap-1 rounded-full border border-areia/14 bg-void/75 px-2 py-0.5 text-[10px] text-areia/85"
                      >
                        <span className="max-w-[120px] truncate font-medium">{f.family}</span>
                        <button
                          type="button"
                          onClick={() =>
                            setDraft((prev) =>
                              prev
                                ? {
                                    ...prev,
                                    tokens: { ...prev.tokens, font_display: f.family },
                                  }
                                : prev,
                            )
                          }
                          className="rounded-full bg-grafite px-1.5 py-0.5 text-[10px] text-amarelo-ideias hover:bg-laranja-criacao/30 hover:text-areia"
                          title={`Set Display Font to ${f.family}`}
                        >
                          Display
                        </button>
                        <button
                          type="button"
                          onClick={() =>
                            setDraft((prev) =>
                              prev
                                ? {
                                    ...prev,
                                    tokens: { ...prev.tokens, font_body: f.family },
                                  }
                                : prev,
                            )
                          }
                          className="rounded-full bg-grafite px-1.5 py-0.5 text-[10px] text-areia/75 hover:bg-laranja-criacao/30 hover:text-areia"
                          title={`Set Body Font to ${f.family}`}
                        >
                          Body
                        </button>
                      </div>
                    ))}
                  </div>
                </div>
              );
            })()}
          </div>
        </div>

        <div className="flex items-center justify-end gap-2.5 border-t border-areia/10 px-5 py-4">
          <button
            type="button"
            onClick={onClose}
            className="rounded-full border border-areia/15 px-4 py-2 text-xs text-areia/80 hover:bg-accent"
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={saving}
            onClick={() => void handleSave()}
            className="btn-solar-outline flex items-center gap-2 px-5 py-2 text-xs font-medium text-areia disabled:opacity-50"
          >
            <Save className="h-3.5 w-3.5 text-amarelo-ideias" />
            {saving ? "Saving..." : "Save Playbook"}
          </button>
        </div>
      </div>
    </div>
  );
}

function RefKindIcon({ kind }: { kind: BrandReferenceKind }) {
  switch (kind) {
    case "reference_video":
      return <Film className="h-4 w-4 text-track-v1" />;
    case "audio_identity":
      return <Music className="h-4 w-4 text-track-a1" />;
    case "lut_cube":
      return <Sparkles className="h-4 w-4 text-laranja-criacao" />;
    case "style_frame":
    case "logo":
      return <ImageIcon className="h-4 w-4 text-track-v2" />;
    default:
      return <FileText className="h-4 w-4 text-amarelo-ideias" />;
  }
}
