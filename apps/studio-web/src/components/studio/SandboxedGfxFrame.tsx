import { useEffect, useRef, useState, type CSSProperties, type SyntheticEvent } from "react";
import type { SolarisEngineClient } from "@solaris/engine-client";

interface SandboxedGfxFrameProps {
  client?: SolarisEngineClient;
  ticketVersion?: number;
  doc: string;
  title: string;
  lazy?: boolean;
  iframeRef?: (el: HTMLIFrameElement | null) => void;
  onLoad?: (e: SyntheticEvent<HTMLIFrameElement>) => void;
  style?: CSSProperties;
  className?: string;
  containerClassName?: string;
  tabIndex?: number;
}

const MAX_CACHE_ENTRIES = 64;
const sandboxUrlCache = new Map<string, Promise<string>>();

function getOrRegisterSandboxUrl(
  client: SolarisEngineClient,
  ticketVersion: number,
  doc: string,
): Promise<string> {
  const key = `${client.getBaseUrl()}:${ticketVersion}:${doc}`;
  const cached = sandboxUrlCache.get(key);
  if (cached) return cached;

  if (sandboxUrlCache.size >= MAX_CACHE_ENTRIES) {
    const oldestKey = sandboxUrlCache.keys().next().value;
    if (oldestKey !== undefined) {
      sandboxUrlCache.delete(oldestKey);
    }
  }

  const pending = client.registerGfxSandbox(doc).catch((err) => {
    sandboxUrlCache.delete(key);
    throw err;
  });
  sandboxUrlCache.set(key, pending);
  return pending;
}

export function SandboxedGfxFrame({
  client,
  ticketVersion = 0,
  doc,
  title,
  lazy = false,
  iframeRef,
  onLoad,
  style,
  className,
  containerClassName = "relative h-full w-full",
  tabIndex,
}: SandboxedGfxFrameProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [visible, setVisible] = useState(!lazy);
  const [sandboxState, setSandboxState] = useState<{
    doc: string;
    ticketVersion: number;
    url: string | null;
    fallbackSrcDoc: boolean;
  }>({
    doc,
    ticketVersion,
    url: null,
    fallbackSrcDoc: !client,
  });

  useEffect(() => {
    if (!lazy || visible) return;
    const el = containerRef.current;
    if (!el || typeof IntersectionObserver === "undefined") {
      setVisible(true);
      return;
    }
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          setVisible(true);
          observer.disconnect();
        }
      },
      { rootMargin: "80px" },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, [lazy, visible]);

  useEffect(() => {
    if (!visible) return;
    if (!client) {
      setSandboxState({ doc, ticketVersion, url: null, fallbackSrcDoc: true });
      return;
    }
    let cancelled = false;
    void getOrRegisterSandboxUrl(client, ticketVersion, doc)
      .then((url) => {
        if (!cancelled) {
          setSandboxState({ doc, ticketVersion, url, fallbackSrcDoc: false });
        }
      })
      .catch(() => {
        if (!cancelled) {
          setSandboxState({ doc, ticketVersion, url: null, fallbackSrcDoc: true });
        }
      });
    return () => {
      cancelled = true;
    };
  }, [client, ticketVersion, doc, visible]);

  const isCurrent =
    sandboxState.doc === doc && sandboxState.ticketVersion === ticketVersion;
  const resolvedUrl = isCurrent ? sandboxState.url : null;
  const useSrcDoc = !client || (isCurrent && sandboxState.fallbackSrcDoc);

  const iframeNode =
    visible && (resolvedUrl || useSrcDoc) ? (
      <iframe
        ref={iframeRef}
        title={title}
        sandbox="allow-scripts"
        src={resolvedUrl ?? undefined}
        srcDoc={resolvedUrl ? undefined : doc}
        onLoad={onLoad}
        style={style}
        className={className}
        tabIndex={tabIndex}
      />
    ) : null;

  if (lazy) {
    return (
      <div ref={containerRef} className={containerClassName}>
        {iframeNode}
      </div>
    );
  }

  return iframeNode;
}
