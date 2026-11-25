import { SolarisEngineClient } from "@solaris/engine-client";

interface TauriEngineConnection {
  url: string;
  token: string;
  desktop_keychain_available?: boolean;
}

declare global {
  interface Window {
    __TAURI_INTERNALS__?: {
      invoke: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
    };
  }
}

export function isDesktopTauri(): boolean {
  return typeof window !== "undefined" && Boolean(window.__TAURI_INTERNALS__?.invoke);
}

export async function saveDesktopOpenRouterKey(apiKey: string): Promise<boolean> {
  if (!isDesktopTauri()) return false;
  try {
    await window.__TAURI_INTERNALS__!.invoke<void>("save_openrouter_key", {
      apiKey,
    });
    return true;
  } catch {
    return false;
  }
}

export async function deleteDesktopOpenRouterKey(): Promise<boolean> {
  if (!isDesktopTauri()) return false;
  try {
    await window.__TAURI_INTERNALS__!.invoke<void>("delete_openrouter_key");
    return true;
  } catch {
    return false;
  }
}

export async function pickDesktopExportFolder(): Promise<string | null> {
  if (!isDesktopTauri()) return null;
  return window.__TAURI_INTERNALS__!.invoke<string | null>("pick_export_folder");
}

export async function saveDesktopExportFile(
  outputPath: string,
  suggestedName?: string,
  targetDir?: string,
): Promise<string | null> {
  if (!isDesktopTauri()) return null;
  return window.__TAURI_INTERNALS__!.invoke<string>("save_export_file", {
    outputPath,
    suggestedName: suggestedName ?? null,
    targetDir: targetDir ?? null,
  });
}

export async function createEngineClient(): Promise<SolarisEngineClient> {
  if (isDesktopTauri()) {
    try {
      const conn = await window.__TAURI_INTERNALS__!.invoke<TauriEngineConnection>(
        "get_engine_connection",
      );
      if (conn?.url) {
        const client = new SolarisEngineClient({
          baseUrl: conn.url,
          authToken: conn.token,
        });
        await client.establishSession().catch(() => {});
        return client;
      }
    } catch {
      // Fallback to default HTTP engine URL if Tauri command is unavailable
    }
  }

  const configured = import.meta.env.VITE_SOLARIS_ENGINE_URL?.trim();
  if (configured) {
    return new SolarisEngineClient({ baseUrl: configured });
  }

  if (
    import.meta.env.PROD &&
    typeof window !== "undefined" &&
    (window.location.protocol === "http:" || window.location.protocol === "https:")
  ) {
    return new SolarisEngineClient({ baseUrl: window.location.origin });
  }

  return new SolarisEngineClient({ baseUrl: "http://127.0.0.1:41731" });
}
