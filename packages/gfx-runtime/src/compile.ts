import { brandTokensToCss, type GfxBrandTokens } from "./tokens.js";
import { GSAP_UMD_SOURCE, SORA_WOFF2_BASE64 } from "./vendorAssets.js";

export function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function resolvePropValue(props: Record<string, unknown>, key: string): unknown {
  if (Object.prototype.hasOwnProperty.call(props, key)) {
    return props[key];
  }
  if (!key.includes(".")) return undefined;
  let cur: unknown = props;
  for (const part of key.split(".")) {
    if (cur && typeof cur === "object" && Object.prototype.hasOwnProperty.call(cur, part)) {
      cur = (cur as Record<string, unknown>)[part];
    } else {
      return undefined;
    }
  }
  return cur;
}

export function interpolateProps(
  htmlSource: string,
  props: Record<string, unknown>,
): string {
  return htmlSource.replace(/\{\{\s*([a-zA-Z0-9_.-]+)\s*\}\}/g, (_, key: string) => {
    const raw = resolvePropValue(props, key);
    if (raw === undefined || raw === null) return "";
    return escapeHtml(String(raw));
  });
}

export interface BuildGfxDocOptions {
  htmlSource: string;
  props: Record<string, unknown>;
  brandTokens: GfxBrandTokens;
  durationSec: number;
  initialTimeSec?: number;
  fontCss?: string;
}

const BUNDLED_SORA_FONT_CSS = `@font-face {
  font-family: "Sora";
  font-style: normal;
  font-weight: 100 800;
  font-display: block;
  src: url("data:font/woff2;base64,${SORA_WOFF2_BASE64}") format("woff2");
}`;

const SAFE_GSAP_SOURCE = GSAP_UMD_SOURCE.replace(/<\/script/gi, "<\\/script");

export function buildSandboxedGfxDoc({
  htmlSource,
  props,
  brandTokens,
  durationSec,
  initialTimeSec = 0,
  fontCss = "",
}: BuildGfxDocOptions): string {
  const bodyContent = interpolateProps(htmlSource, props);
  const cssTokens = brandTokensToCss(brandTokens);
  const finiteDuration = Number.isFinite(durationSec) && durationSec > 0 ? durationSec : 3;
  const safeDuration = Math.max(0.1, finiteDuration);
  const finiteInitial = Number.isFinite(initialTimeSec) && initialTimeSec >= 0 ? initialTimeSec : 0;
  const safeInitial = Math.max(0, Math.min(safeDuration, finiteInitial));
  let safeFontCss = fontCss;
  let prevFontCss: string;
  do {
    prevFontCss = safeFontCss;
    safeFontCss = safeFontCss.replace(/<\s*\/\s*style/gi, "");
  } while (safeFontCss !== prevFontCss);

  return `<!doctype html>
<html>
<head>
<meta charset="utf-8" />
<meta name="color-scheme" content="dark" />
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src data:; font-src data:;" />
<style>
${BUNDLED_SORA_FONT_CSS}
${safeFontCss}
${cssTokens}
* { box-sizing: border-box; margin: 0; padding: 0; }
html, body {
  color-scheme: dark;
  width: 100%;
  height: 100%;
  background: transparent !important;
  overflow: hidden;
  display: grid;
  place-items: center;
  font-family: var(--brand-font-body);
  color: var(--brand-ink);
}
</style>
<script>${SAFE_GSAP_SOURCE}</script>
</head>
<body>
${bodyContent}
<script>
(function() {
  const DURATION = ${safeDuration};
  let lastSeekTime = ${safeInitial};
  function seekAnimations(t) {
    const raw = Number(t);
    const clamped = Math.max(0, Math.min(DURATION, Number.isFinite(raw) ? raw : 0));
    lastSeekTime = clamped;
    const ms = clamped * 1000;
    if (typeof document.getAnimations === "function") {
      for (const anim of document.getAnimations()) {
        try {
          anim.pause();
          anim.currentTime = ms;
        } catch (_) {}
      }
    }
    if (window.gsap && window.gsap.globalTimeline && typeof window.gsap.globalTimeline.pause === "function") {
      try {
        window.gsap.globalTimeline.pause(clamped, false);
      } catch (_) {}
    }
    if (window.__timelines && typeof window.__timelines === "object") {
      for (const tl of Object.values(window.__timelines)) {
        if (tl && typeof tl.pause === "function" && typeof tl.seek === "function") {
          tl.pause();
          tl.seek(clamped, false);
        }
      }
    }
    if (typeof window.__solarisOnSeek === "function") {
      window.__solarisOnSeek(clamped, DURATION);
    }
  }
  window.__solarisSeek = seekAnimations;
  window.addEventListener("message", function(ev) {
    if (!ev || ev.source !== window.parent) return;
    if (ev.data && ev.data.type === "solaris:seek") {
      seekAnimations(ev.data.time);
    }
  });
  seekAnimations(lastSeekTime);
  document.addEventListener("DOMContentLoaded", function() {
    seekAnimations(lastSeekTime);
  });
  window.addEventListener("load", function() {
    seekAnimations(lastSeekTime);
  });
  if (window.parent && window.parent !== window) {
    window.parent.postMessage({ type: "solaris:gfx-ready" }, "*");
  }
})();
</script>
</body>
</html>`;
}
