import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type RefObject,
} from "react";
import { clampToViewport, type Point } from "../../lib/gfxAssetActions";

export type GfxMenuAction = "add" | "rename" | "duplicate" | "export" | "delete";

// `local` actions never touch the server, so they stay enabled while a mutation runs.
const MENU_ITEMS: Array<{ action: GfxMenuAction; label: string; local?: boolean }> = [
  { action: "add", label: "Add at playhead", local: true },
  { action: "rename", label: "Rename" },
  { action: "duplicate", label: "Duplicate" },
  { action: "export", label: "Export JSON", local: true },
  { action: "delete", label: "Delete" },
];

const ITEM_SELECTOR = "[role=menuitem]:not(:disabled)";

interface GfxAssetMenuProps {
  assetName: string;
  x: number;
  y: number;
  align: "start" | "end";
  busy: boolean;
  triggerRef: RefObject<HTMLButtonElement | null>;
  onSelect: (action: GfxMenuAction, keyboard: boolean) => void;
  onClose: (restoreFocus: boolean) => void;
}

export function GfxAssetMenu({
  assetName,
  x,
  y,
  align,
  busy,
  triggerRef,
  onSelect,
  onClose,
}: GfxAssetMenuProps) {
  const menuRef = useRef<HTMLDivElement | null>(null);
  const [origin, setOrigin] = useState<Point | null>(null);

  useLayoutEffect(() => {
    const menu = menuRef.current;
    if (!menu) return;
    const size = { width: menu.offsetWidth, height: menu.offsetHeight };
    const left = align === "end" ? x - size.width : x;
    setOrigin(
      clampToViewport({ x: left, y }, size, {
        width: window.innerWidth,
        height: window.innerHeight,
      }),
    );
  }, [x, y, align]);

  useEffect(() => {
    if (origin) {
      menuRef.current?.querySelector<HTMLButtonElement>(ITEM_SELECTOR)?.focus({ preventScroll: true });
    }
  }, [origin]);

  useEffect(() => {
    const dismissOnOutsidePointer = (event: PointerEvent) => {
      const target = event.target;
      if (
        target instanceof Node &&
        (menuRef.current?.contains(target) || triggerRef.current?.contains(target))
      ) {
        return;
      }
      onClose(false);
    };
    const dismiss = () => onClose(false);
    document.addEventListener("pointerdown", dismissOnOutsidePointer);
    window.addEventListener("resize", dismiss);
    window.addEventListener("scroll", dismiss, true);
    return () => {
      document.removeEventListener("pointerdown", dismissOnOutsidePointer);
      window.removeEventListener("resize", dismiss);
      window.removeEventListener("scroll", dismiss, true);
    };
  }, [onClose, triggerRef]);

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    event.stopPropagation();
    const items = Array.from(
      menuRef.current?.querySelectorAll<HTMLButtonElement>(ITEM_SELECTOR) ?? [],
    );
    const index = items.indexOf(document.activeElement as HTMLButtonElement);
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        items[(index + 1) % items.length]?.focus();
        break;
      case "ArrowUp":
        event.preventDefault();
        items[index <= 0 ? items.length - 1 : index - 1]?.focus();
        break;
      case "Home":
        event.preventDefault();
        items[0]?.focus();
        break;
      case "End":
        event.preventDefault();
        items[items.length - 1]?.focus();
        break;
      case "Escape":
        event.preventDefault();
        onClose(true);
        break;
      case "Tab":
        onClose(false);
        break;
    }
  };

  return (
    <div
      ref={menuRef}
      role="menu"
      aria-label={`Actions for ${assetName}`}
      onKeyDown={handleKeyDown}
      onContextMenu={(event) => {
        event.preventDefault();
        event.stopPropagation();
      }}
      style={origin ? { left: origin.x, top: origin.y } : { left: 0, top: 0, visibility: "hidden" }}
      className="fixed z-50 flex min-w-44 flex-col gap-0.5 rounded-xl border border-areia/14 bg-carbono/95 p-1 shadow-2xl backdrop-blur-md"
    >
      {MENU_ITEMS.map((item) => (
        <button
          key={item.action}
          type="button"
          role="menuitem"
          disabled={busy && !item.local}
          onClick={(event) => onSelect(item.action, event.detail === 0)}
          className="flex items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-[11px] text-areia hover:bg-accent focus-visible:bg-accent disabled:opacity-45"
        >
          {item.action === "delete" && (
            <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full bg-vermelho-foco" />
          )}
          {item.label}
        </button>
      ))}
    </div>
  );
}
