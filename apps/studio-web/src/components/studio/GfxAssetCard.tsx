import {
  useEffect,
  useId,
  useRef,
  useState,
  type CSSProperties,
  type DragEvent,
  type FormEvent,
  type KeyboardEvent,
  type MouseEvent,
} from "react";
import { Plus } from "lucide-react";
import type { SavedCodeAsset, SolarisEngineClient } from "@solaris/engine-client";
import { GFX_ASSET_DRAG_MIME } from "../../lib/dragPayloads";
import { checkGfxName, type GfxPreviewGeometry } from "../../lib/gfxAssetActions";
import { GfxAssetMenu, type GfxMenuAction } from "./GfxAssetMenu";
import { SandboxedGfxFrame } from "./SandboxedGfxFrame";

interface MenuState {
  x: number;
  y: number;
  align: "start" | "end";
  source: "more" | "primary";
}

export interface GfxAssetCardProps {
  asset: SavedCodeAsset;
  client: SolarisEngineClient;
  ticketVersion: number;
  doc: string;
  geometry: GfxPreviewGeometry;
  playheadSec: number;
  busy: boolean;
  onAdd: () => void;
  onRename: (name: string) => Promise<boolean>;
  onDuplicate: () => void;
  onExport: () => void;
  onDelete: () => Promise<boolean>;
}

export function GfxAssetCard({
  asset,
  client,
  ticketVersion,
  doc,
  geometry,
  playheadSec,
  busy,
  onAdd,
  onRename,
  onDuplicate,
  onExport,
  onDelete,
}: GfxAssetCardProps) {
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [edit, setEdit] = useState<"rename" | "delete" | null>(null);
  const [draft, setDraft] = useState(asset.name);
  const restoreFocusRef = useRef(false);
  const primaryRef = useRef<HTMLButtonElement | null>(null);
  const moreRef = useRef<HTMLButtonElement | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const cancelRef = useRef<HTMLButtonElement | null>(null);
  const inputId = useId();
  const errorId = useId();
  const confirmId = useId();
  const nameCheck = checkGfxName(draft);

  useEffect(() => {
    if (edit === "rename") {
      inputRef.current?.focus();
      inputRef.current?.select();
    } else if (edit === "delete") {
      cancelRef.current?.focus();
    } else if (restoreFocusRef.current) {
      restoreFocusRef.current = false;
      primaryRef.current?.focus();
    }
  }, [edit]);

  const openMenu = (x: number, y: number, align: MenuState["align"], source: MenuState["source"]) =>
    setMenu({ x, y, align, source });

  const focusMenuTrigger = () => (menu?.source === "more" ? moreRef : primaryRef).current?.focus();

  const closeMenu = (restoreFocus: boolean) => {
    if (restoreFocus) focusMenuTrigger();
    setMenu(null);
  };

  const closeEdit = () => {
    restoreFocusRef.current = true;
    setEdit(null);
  };

  // Keyboard activations return focus to the trigger; pointer activations leave it alone so
  // a follow-up Space keypress is not swallowed by a focused card button.
  const handleSelect = (action: GfxMenuAction, keyboard: boolean) => {
    setMenu(null);
    if (action === "rename") {
      setDraft(asset.name);
      setEdit("rename");
      return;
    }
    if (action === "delete") {
      setEdit("delete");
      return;
    }
    if (keyboard) focusMenuTrigger();
    switch (action) {
      case "add":
        onAdd();
        break;
      case "duplicate":
        onDuplicate();
        break;
      case "export":
        onExport();
        break;
    }
  };

  const handleMoreClick = () => {
    if (menu) {
      closeMenu(true);
      return;
    }
    const rect = moreRef.current?.getBoundingClientRect();
    if (rect) openMenu(rect.right, rect.bottom + 6, "end", "more");
  };

  const handleContextMenu = (event: MouseEvent<HTMLLIElement>) => {
    if (edit) return;
    event.preventDefault();
    openMenu(event.clientX, event.clientY, "start", "primary");
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLLIElement>) => {
    const opensMenu = event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey);
    if (!opensMenu || edit) return;
    event.preventDefault();
    const rect = primaryRef.current?.getBoundingClientRect();
    if (rect) openMenu(rect.left + 12, rect.bottom, "start", "primary");
  };

  const handleDragStart = (event: DragEvent<HTMLLIElement>) => {
    event.dataTransfer.effectAllowed = "copy";
    event.dataTransfer.setData(GFX_ASSET_DRAG_MIME, asset.id);
    event.dataTransfer.setData("text/plain", asset.id);
  };

  const handleEditKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (event.key !== "Escape") return;
    event.preventDefault();
    event.stopPropagation();
    closeEdit();
  };

  const submitRename = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!nameCheck.ok) return;
    const { name } = nameCheck;
    if (name !== asset.name && !(await onRename(name))) return;
    closeEdit();
  };

  const confirmDelete = async () => {
    if (!(await onDelete())) closeEdit();
  };

  return (
    <li
      draggable={edit === null}
      onDragStart={handleDragStart}
      onContextMenu={handleContextMenu}
      onKeyDown={handleKeyDown}
      className={`group relative flex min-w-0 flex-col gap-1 rounded-xl border border-areia/10 bg-void/70 p-1.5 transition-colors hover:border-track-g1/50 motion-reduce:transition-none ${
        edit ? "col-span-2 border-track-g1/50" : ""
      }`}
    >
      {edit === "rename" && (
        <form
          onSubmit={(event) => void submitRename(event)}
          onKeyDown={handleEditKeyDown}
          className="flex flex-col gap-1.5 rounded-lg border border-areia/12 bg-carbono/90 p-2"
        >
          <label htmlFor={inputId} className="text-[10px] font-semibold uppercase tracking-[0.12em] text-areia/65">
            Rename GFX
          </label>
          <input
            id={inputId}
            ref={inputRef}
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            aria-invalid={!nameCheck.ok}
            aria-describedby={errorId}
            className="h-7 rounded-lg border border-areia/20 bg-void px-2 text-[11px] text-areia focus:border-laranja-criacao"
          />
          <p id={errorId} className="min-h-3 text-[10px] text-areia/80">
            {nameCheck.ok ? "" : nameCheck.message}
          </p>
          <div className="flex justify-end gap-1">
            <button
              type="button"
              onClick={closeEdit}
              className="rounded-full border border-areia/15 px-2.5 py-1 text-[10px] text-areia/85 hover:bg-accent"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={busy || !nameCheck.ok}
              className="rounded-full border border-amarelo-ideias/40 px-2.5 py-1 text-[10px] text-areia hover:bg-accent disabled:opacity-45"
            >
              Save
            </button>
          </div>
        </form>
      )}

      {edit === "delete" && (
        <div
          role="group"
          aria-labelledby={confirmId}
          onKeyDown={handleEditKeyDown}
          className="flex flex-col gap-1.5 rounded-lg border border-vermelho-foco/40 bg-carbono/90 p-2"
        >
          <p id={confirmId} className="text-[11px] leading-snug text-areia">
            Delete “{asset.name}”? Timeline clips keep their copy.
          </p>
          <div className="flex justify-end gap-1">
            <button
              type="button"
              disabled={busy}
              onClick={() => void confirmDelete()}
              className="rounded-full border border-vermelho-foco/40 bg-vermelho-foco/15 px-2.5 py-1 text-[10px] text-areia hover:bg-vermelho-foco/25 disabled:opacity-45"
            >
              Delete
            </button>
            <button
              ref={cancelRef}
              type="button"
              onClick={closeEdit}
              className="rounded-full border border-areia/15 px-2.5 py-1 text-[10px] text-areia/85 hover:bg-accent"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {edit === null && (
        <>
          <button
            ref={primaryRef}
            type="button"
            onClick={onAdd}
            aria-label={`Add ${asset.name} at ${playheadSec.toFixed(1)} s`}
            className="flex min-w-0 flex-col gap-1 rounded-lg text-left transition-colors hover:bg-accent motion-reduce:transition-none"
          >
            <div
              aria-hidden="true"
              style={{ height: geometry.boxHeightPx }}
              className="flex items-center justify-center overflow-hidden rounded-lg border border-areia/10 bg-carbono/90"
            >
              <div className="relative h-full" style={{ width: geometry.boxWidthPx }}>
                <SandboxedGfxFrame
                  client={client}
                  ticketVersion={ticketVersion}
                  title={`Preview ${asset.name}`}
                  doc={doc}
                  lazy
                  tabIndex={-1}
                  style={previewFrameStyle(geometry)}
                  className="pointer-events-none absolute top-1/2 left-1/2 border-0 bg-transparent"
                />
              </div>
            </div>
            <span className="flex min-w-0 items-center justify-between gap-1 px-1 text-[10px] text-areia/85">
              <span className="truncate font-medium">{asset.name}</span>
              <Plus aria-hidden="true" className="h-3 w-3 shrink-0 text-track-g1" />
            </span>
            <span className="truncate px-1 text-[10px] text-areia/65">{asset.category}</span>
          </button>
          <button
            ref={moreRef}
            type="button"
            onClick={handleMoreClick}
            aria-label={`More actions for ${asset.name}`}
            aria-haspopup="menu"
            aria-expanded={menu !== null}
            className={`absolute top-3 right-3 z-10 grid h-6 w-6 place-items-center rounded-full border border-areia/15 bg-carbono/90 text-xs leading-none text-areia/85 transition-opacity hover:border-amarelo-ideias/50 hover:text-areia focus-visible:opacity-100 group-hover:opacity-100 group-focus-within:opacity-100 pointer-coarse:opacity-100 motion-reduce:transition-none ${
              menu ? "opacity-100" : "opacity-0"
            }`}
          >
            ⋯
          </button>
        </>
      )}

      {menu && (
        <GfxAssetMenu
          assetName={asset.name}
          x={menu.x}
          y={menu.y}
          align={menu.align}
          busy={busy}
          triggerRef={moreRef}
          onSelect={handleSelect}
          onClose={closeMenu}
        />
      )}
    </li>
  );
}

function previewFrameStyle(geometry: GfxPreviewGeometry): CSSProperties {
  return {
    width: geometry.viewportWidthPx,
    height: geometry.viewportHeightPx,
    transform: `translate(-50%, -50%) scale(${geometry.scale})`,
  };
}
