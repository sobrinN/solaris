import { useEffect, useRef } from "react";
import type {
  EngineEventEnvelope,
  SavedCodeAsset,
  SolarisEngineClient,
} from "@solaris/engine-client";
import { errorMessage } from "../../lib/gfxAssetActions";

const ASSET_REFRESH_DEBOUNCE_MS = 150;

interface GfxAssetSyncOptions {
  client: SolarisEngineClient;
  events: EngineEventEnvelope[];
  onAssets: (assets: SavedCodeAsset[]) => void;
  onError: (message: string) => void;
}

// `events` is a capped window, so new envelopes are found by object identity, not by length.
export function useGfxAssetSync({ client, events, onAssets, onError }: GfxAssetSyncOptions): void {
  const lastSeenRef = useRef<EngineEventEnvelope | undefined>(events.at(-1));
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const requestSeqRef = useRef(0);
  const handlersRef = useRef({ onAssets, onError });

  useEffect(() => {
    handlersRef.current = { onAssets, onError };
  });

  useEffect(
    () => () => {
      if (timerRef.current !== null) clearTimeout(timerRef.current);
    },
    [],
  );

  useEffect(() => {
    let assetsChanged = false;
    for (let i = events.length - 1; i >= 0 && events[i] !== lastSeenRef.current; i -= 1) {
      if (events[i].payload.event === "assets_changed") assetsChanged = true;
    }
    lastSeenRef.current = events.at(-1);
    if (!assetsChanged) return;

    if (timerRef.current !== null) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => {
      timerRef.current = null;
      const seq = ++requestSeqRef.current;
      void client.listAssets().then(
        (assets) => {
          if (seq === requestSeqRef.current) handlersRef.current.onAssets(assets);
        },
        (err: unknown) => {
          if (seq === requestSeqRef.current) {
            handlersRef.current.onError(errorMessage(err, "Failed to refresh the GFX library"));
          }
        },
      );
    }, ASSET_REFRESH_DEBOUNCE_MS);
  }, [client, events]);
}
