import { useEffect, useMemo, useRef, useState } from "react";
import { Download, Layers, Upload } from "lucide-react";
import type {
  BrandTokens,
  CodeAssetPackManifest,
  SavedCodeAsset,
  SolarisEngineClient,
} from "@solaris/engine-client";
import { buildSandboxedGfxDoc } from "@solaris/gfx-runtime";
import {
  duplicateGfxName,
  errorMessage,
  gfxPreviewGeometry,
  missingStarterGfxIds,
  nextDuplicateId,
  type GfxPreviewAspect,
} from "../../lib/gfxAssetActions";
import {
  ALL_GFX_CATEGORIES,
  filterGfxAssets,
  gfxCategories,
  resolveGfxCategory,
  type GfxCategory,
} from "../../lib/gfxBinFilters";
import { GfxAssetCard } from "./GfxAssetCard";

export interface GfxBinPanelProps {
  client: SolarisEngineClient;
  projectId: string;
  ticketVersion: number;
  assets: SavedCodeAsset[];
  aspect: GfxPreviewAspect;
  brandTokens: BrandTokens;
  brandFontCss: string;
  playheadSec: number;
  onAddAtPlayhead: (asset: SavedCodeAsset) => void;
  onAssetsChanged: (assets: SavedCodeAsset[]) => void;
  onNotice: (message: string) => void;
  onError: (message: string) => void;
}

export function GfxBinPanel({
  client,
  projectId,
  ticketVersion,
  assets,
  aspect,
  brandTokens,
  brandFontCss,
  playheadSec,
  onAddAtPlayhead,
  onAssetsChanged,
  onNotice,
  onError,
}: GfxBinPanelProps) {
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState(ALL_GFX_CATEGORIES);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const packInputRef = useRef<HTMLInputElement | null>(null);
  const listRef = useRef<HTMLUListElement | null>(null);

  useEffect(() => {
    setQuery("");
    setCategory(ALL_GFX_CATEGORIES);
  }, [projectId]);

  const categories = useMemo(() => gfxCategories(assets), [assets]);
  const activeCategory = resolveGfxCategory(categories, category);
  const visible = useMemo(
    () => filterGfxAssets(assets, { query, category: activeCategory }),
    [assets, query, activeCategory],
  );
  const missingStarters = useMemo(() => missingStarterGfxIds(assets), [assets]);
  const geometry = useMemo(() => gfxPreviewGeometry(aspect), [aspect]);
  const previewDocs = useMemo(
    () => buildPreviewDocs(assets, brandTokens, brandFontCss),
    [assets, brandTokens, brandFontCss],
  );
  const pills: GfxCategory[] = [{ name: ALL_GFX_CATEGORIES, count: assets.length }, ...categories];

  const runAction = async (task: () => Promise<string | null>, failure: string): Promise<boolean> => {
    if (busyRef.current) return false;
    busyRef.current = true;
    setBusy(true);
    try {
      const notice = await task();
      if (notice) onNotice(notice);
      return true;
    } catch (err) {
      onError(errorMessage(err, failure));
      return false;
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  };

  const refreshAssets = async () => onAssetsChanged(await client.listAssets());

  const renameAsset = (asset: SavedCodeAsset, name: string) =>
    runAction(async () => {
      await client.saveAsset({ ...asset, name });
      await refreshAssets();
      return `Renamed to “${name}”.`;
    }, "Failed to rename GFX");

  const duplicateAsset = (asset: SavedCodeAsset) =>
    runAction(async () => {
      const taken = (await client.listAssets()).map((item) => item.id);
      const copy = {
        ...asset,
        id: nextDuplicateId(asset.id, taken),
        name: duplicateGfxName(asset.name),
      };
      await client.saveAsset(copy);
      await refreshAssets();
      return `Duplicated as “${copy.name}”.`;
    }, "Failed to duplicate GFX");

  const removeAsset = (asset: SavedCodeAsset) =>
    runAction(async () => {
      await client.deleteAsset(asset.id);
      listRef.current?.focus({ preventScroll: true });
      await refreshAssets();
      return `Deleted “${asset.name}”.`;
    }, "Failed to delete GFX");

  const restoreStarters = () =>
    runAction(async () => {
      const res = await client.restoreStarterAssets();
      await refreshAssets();
      return res.restored_count > 0
        ? `Restored ${res.restored_count} starter GFX.`
        : "Starter GFX are already in the library.";
    }, "Failed to restore starter GFX");

  const importPack = (fileList: FileList | null) => {
    const file = fileList?.[0];
    if (!file) return;
    void runAction(async () => {
      const manifest = JSON.parse(await file.text()) as CodeAssetPackManifest;
      const res = await client.importAssetPack(manifest);
      await refreshAssets();
      return `Imported ${res.imported_count} GFX.`;
    }, "Failed to import GFX pack");
  };

  const exportPack = async () => {
    try {
      downloadJson(await client.exportAssetPack(), "solaris-asset-pack.json");
    } catch (err) {
      onError(errorMessage(err, "Failed to export GFX pack"));
    }
  };

  const exportAssetJson = (asset: SavedCodeAsset) => {
    try {
      downloadJson(asset, `${asset.id}.json`);
    } catch (err) {
      onError(errorMessage(err, "Failed to export GFX"));
    }
  };

  const clearFilters = () => {
    setQuery("");
    setCategory(ALL_GFX_CATEGORIES);
  };

  return (
    <section
      aria-label="GFX library"
      data-cursor-anchor="effects.library"
      className="flex min-h-0 flex-1 flex-col rounded-2xl border border-areia/10 bg-carbono/70 p-3"
    >
      <div className="mb-2 flex flex-wrap items-center justify-between gap-1.5">
        <div className="flex items-center gap-1.5">
          <Layers aria-hidden="true" className="h-3 w-3 text-track-g1" />
          <h2 className="text-[10px] font-semibold uppercase tracking-[0.14em] text-areia/60">
            GFX ({assets.length})
          </h2>
        </div>
        <div className="flex items-center gap-1">
          <input
            ref={packInputRef}
            type="file"
            accept="application/json,.json"
            onChange={(event) => {
              importPack(event.target.files);
              event.currentTarget.value = "";
            }}
            className="hidden"
          />
          <button
            type="button"
            disabled={busy}
            onClick={() => packInputRef.current?.click()}
            aria-label="Import GFX pack"
            title="Import solaris.asset-pack/v1 JSON"
            className="flex items-center gap-1 rounded-full border border-areia/12 px-2 py-0.5 text-[10px] text-areia/80 hover:bg-accent disabled:opacity-45"
          >
            <Upload aria-hidden="true" className="h-2.5 w-2.5 text-amarelo-ideias" />
            <span>Import</span>
          </button>
          <button
            type="button"
            onClick={() => void exportPack()}
            aria-label="Export GFX pack"
            title="Export workspace GFX as solaris.asset-pack/v1 JSON"
            className="flex items-center gap-1 rounded-full border border-areia/12 px-2 py-0.5 text-[10px] text-areia/80 hover:bg-accent"
          >
            <Download aria-hidden="true" className="h-2.5 w-2.5 text-laranja-criacao" />
            <span>Export</span>
          </button>
        </div>
      </div>

      <div className="mb-2 flex flex-col gap-1.5">
        <input
          type="search"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Search name, id, or tag"
          aria-label="Search GFX by name, id, or tag"
          className="h-7 w-full rounded-lg border border-areia/12 bg-void/70 px-2.5 text-[11px] text-areia placeholder:text-areia/50 focus:border-laranja-criacao/60"
        />
        <div role="group" aria-label="Filter GFX by category" className="flex flex-wrap gap-1">
          {pills.map((pill) => {
            const selected = pill.name === activeCategory;
            return (
              <button
                key={`pill:${pill.name}`}
                type="button"
                aria-pressed={selected}
                onClick={() => setCategory(pill.name)}
                className={`rounded-full border px-2 py-0.5 text-[10px] transition-colors motion-reduce:transition-none ${
                  selected
                    ? "border-laranja-criacao/60 bg-accent font-semibold text-areia"
                    : "border-areia/12 text-areia/75 hover:text-areia"
                }`}
              >
                {pill.name === ALL_GFX_CATEGORIES ? "All" : pill.name}{" "}
                <span className="tabular-nums text-areia/60">{pill.count}</span>
              </button>
            );
          })}
        </div>
      </div>

      <ul
        ref={listRef}
        aria-label="GFX assets"
        tabIndex={-1}
        className="grid min-h-0 flex-1 grid-cols-2 content-start gap-1.5 overflow-y-auto pr-0.5"
      >
        {visible.map((asset) => (
          <GfxAssetCard
            key={asset.id}
            asset={asset}
            client={client}
            ticketVersion={ticketVersion}
            doc={previewDocs[asset.id]}
            geometry={geometry}
            playheadSec={playheadSec}
            busy={busy}
            onAdd={() => onAddAtPlayhead(asset)}
            onRename={(name) => renameAsset(asset, name)}
            onDuplicate={() => void duplicateAsset(asset)}
            onExport={() => exportAssetJson(asset)}
            onDelete={() => removeAsset(asset)}
          />
        ))}
      </ul>

      {visible.length === 0 && (
        <div className="mt-2 rounded-xl border border-dashed border-areia/12 p-4 text-center text-[11px] text-areia/70">
          <p className="font-medium text-areia/85">
            {assets.length === 0 ? "No GFX saved yet." : "No GFX match"}
          </p>
          <p className="mt-1">
            {assets.length === 0
              ? "Save a clip as GFX, import a pack, or restore the starters."
              : "Try another name, id, or tag."}
          </p>
          {assets.length > 0 && (
            <button
              type="button"
              onClick={clearFilters}
              className="mt-2 rounded-full border border-areia/15 px-2.5 py-1 text-[10px] text-areia/85 hover:bg-accent"
            >
              Clear search
            </button>
          )}
        </div>
      )}

      {missingStarters.length > 0 && (
        <div className="mt-2 flex items-center justify-between gap-2 border-t border-areia/10 pt-2">
          <span className="tabular-nums text-[10px] text-areia/70">
            {missingStarters.length} starter GFX deleted
          </span>
          <button
            type="button"
            disabled={busy}
            onClick={() => void restoreStarters()}
            className="rounded-full border border-amarelo-ideias/40 px-2.5 py-1 text-[10px] text-areia hover:bg-accent disabled:opacity-45"
          >
            Restore starter GFX
          </button>
        </div>
      )}
    </section>
  );
}

function buildPreviewDocs(
  assets: readonly SavedCodeAsset[],
  brandTokens: BrandTokens,
  fontCss: string,
): Record<string, string> {
  const docs: Record<string, string> = {};
  for (const asset of assets) {
    const duration = asset.default_duration_sec || 2.5;
    docs[asset.id] = buildSandboxedGfxDoc({
      htmlSource: asset.html_source,
      props: asset.default_props,
      brandTokens,
      fontCss,
      durationSec: duration,
      initialTimeSec: Math.min(0.8, duration * 0.4),
    });
  }
  return docs;
}

function downloadJson(payload: unknown, fileName: string): void {
  const url = URL.createObjectURL(
    new Blob([JSON.stringify(payload, null, 2)], { type: "application/json" }),
  );
  const link = document.createElement("a");
  link.href = url;
  link.download = fileName;
  document.body.appendChild(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
