export interface GfxLintIssue {
  code: string;
  severity: "error" | "warning";
  message: string;
}

interface ParsedTag {
  name: string;
  attrs: Array<[string, string]>;
}

interface ScannedHtml {
  tags: ParsedTag[];
  scriptSources: string[];
  styleSources: string[];
}

function isExternalUrl(raw: string): boolean {
  const trimmed = raw.trim().toLowerCase();
  return trimmed.length > 0 && !trimmed.startsWith("data:") && !trimmed.startsWith("#");
}

function hasExternalSrcsetUrl(raw: string): boolean {
  const len = raw.length;
  let i = 0;

  while (i < len) {
    while (i < len && (raw[i] === "," || /\s/.test(raw[i]))) i += 1;
    if (i >= len) break;

    const start = i;
    const isDataUri = raw.slice(i, i + 5).toLowerCase() === "data:";
    let seenDataComma = false;

    while (i < len && !/\s/.test(raw[i])) {
      if (raw[i] === ",") {
        if (isDataUri && !seenDataComma) {
          seenDataComma = true;
          i += 1;
          continue;
        }
        break;
      }
      i += 1;
    }

    const urlToken = raw.slice(start, i);
    if (isExternalUrl(urlToken)) {
      return true;
    }

    while (i < len && raw[i] !== ",") i += 1;
    if (i < len && raw[i] === ",") i += 1;
  }

  return false;
}

function stripCssComments(css: string): string {
  return css.replace(/\/\*[\s\S]*?\*\//g, "");
}

function extractCssUrls(css: string): string[] {
  const urls: string[] = [];
  const re = /(?:^|[^A-Za-z0-9_-])url\s*\(/gi;
  let match: RegExpExecArray | null;

  while ((match = re.exec(css)) !== null) {
    let i = match.index + match[0].length;
    while (i < css.length && /\s/.test(css[i])) i += 1;
    let raw = "";

    if (i < css.length && (css[i] === '"' || css[i] === "'")) {
      const quote = css[i];
      i += 1;
      const start = i;
      while (i < css.length && css[i] !== quote) {
        if (css[i] === "\\" && i + 1 < css.length) {
          i += 2;
        } else {
          i += 1;
        }
      }
      raw = css.slice(start, i);
      if (i < css.length) i += 1;
    } else {
      const start = i;
      while (i < css.length && css[i] !== ")") i += 1;
      raw = css.slice(start, i);
    }

    urls.push(raw.trim());
    re.lastIndex = i;
  }

  return urls;
}

function scanHtml(html: string): ScannedHtml {
  const tags: ParsedTag[] = [];
  const scriptSources: string[] = [];
  const styleSources: string[] = [];
  const len = html.length;
  let i = 0;

  while (i < len) {
    if (html[i] !== "<") {
      i += 1;
      continue;
    }
    if (html.startsWith("<!--", i)) {
      const end = html.indexOf("-->", i + 4);
      if (end === -1) break;
      i = end + 3;
      continue;
    }
    i += 1;
    if (i >= len || html[i] === "/" || html[i] === "!" || html[i] === "?") {
      continue;
    }
    const nameStart = i;
    while (i < len && /[a-zA-Z0-9-]/.test(html[i])) {
      i += 1;
    }
    if (i === nameStart) continue;
    const tagName = html.slice(nameStart, i).toLowerCase();
    const attrs: Array<[string, string]> = [];
    let selfClosing = false;

    while (i < len) {
      while (i < len && /\s/.test(html[i])) i += 1;
      if (i >= len || html[i] === ">") {
        i = Math.min(i + 1, len);
        break;
      }
      if (html[i] === "/") {
        i += 1;
        if (i < len && html[i] === ">") {
          selfClosing = true;
          i += 1;
          break;
        }
        continue;
      }
      const attrStart = i;
      while (i < len && !/[\s=>/]/.test(html[i])) i += 1;
      const attrName = html.slice(attrStart, i).toLowerCase();
      while (i < len && /\s/.test(html[i])) i += 1;
      let attrVal = "";
      if (i < len && html[i] === "=") {
        i += 1;
        while (i < len && /\s/.test(html[i])) i += 1;
        if (i < len && (html[i] === '"' || html[i] === "'")) {
          const quote = html[i];
          i += 1;
          const valStart = i;
          while (i < len && html[i] !== quote) i += 1;
          attrVal = html.slice(valStart, i);
          if (i < len) i += 1;
        } else {
          const valStart = i;
          while (i < len && !/[\s>]/.test(html[i])) i += 1;
          attrVal = html.slice(valStart, i);
        }
      }
      if (attrName) {
        attrs.push([attrName, attrVal]);
        if (attrName === "style" && attrVal.trim().length > 0) {
          styleSources.push(attrVal);
        } else if (attrName.length > 2 && /^on[a-z]/.test(attrName) && attrVal.trim().length > 0) {
          scriptSources.push(attrVal);
        }
      }
    }

    tags.push({ name: tagName, attrs });

    if (tagName === "script" || tagName === "style") {
      const closeRe =
        tagName === "script" ? /<\/script(?=[\s/>]|$)/gi : /<\/style(?=[\s/>]|$)/gi;
      closeRe.lastIndex = i;
      const match = closeRe.exec(html);
      if (match) {
        const inner = html.slice(i, match.index);
        if (inner.length > 0) {
          if (tagName === "script") {
            scriptSources.push(inner);
          } else {
            styleSources.push(inner);
          }
        }
        const gt = html.indexOf(">", match.index + match[0].length);
        i = gt === -1 ? len : gt + 1;
      } else if (!selfClosing) {
        const inner = html.slice(i);
        if (inner.length > 0) {
          if (tagName === "script") {
            scriptSources.push(inner);
          } else {
            styleSources.push(inner);
          }
        }
        i = len;
      }
    }
  }

  return { tags, scriptSources, styleSources };
}

const CSS_IMPORT_RE = /(?:^|[^A-Za-z0-9_-])@import(?=$|[^A-Za-z0-9_-])/i;

function findMatchingBrace(s: string, openBrace: number): number {
  let depth = 0;
  for (let i = openBrace; i < s.length; i += 1) {
    if (s[i] === "{") depth += 1;
    else if (s[i] === "}") {
      depth -= 1;
      if (depth === 0) return i;
    }
  }
  return -1;
}

function stripImportant(val: string): string {
  const trimmed = val.trim();
  if (trimmed.endsWith("!important")) {
    return trimmed.slice(0, -"!important".length).trimEnd();
  }
  return trimmed;
}

function hasDescendantSvgAbsoluteTrap(css: string): boolean {
  const lower = css.toLowerCase();
  let cursor = 0;
  while (cursor < lower.length) {
    const open = lower.indexOf("{", cursor);
    if (open === -1) break;
    const preludeSegment = lower.slice(cursor, open);
    const semi = preludeSegment.indexOf(";");
    if (semi !== -1 && preludeSegment.slice(0, semi).trimStart().startsWith("@")) {
      cursor = cursor + semi + 1;
      continue;
    }
    const close = findMatchingBrace(lower, open);
    if (close === -1) break;
    const selectorBlock = preludeSegment.trim();
    const body = lower.slice(open + 1, close);
    cursor = close + 1;

    if (selectorBlock.startsWith("@")) {
      if (
        selectorBlock.startsWith("@media") ||
        selectorBlock.startsWith("@supports") ||
        selectorBlock.startsWith("@layer") ||
        selectorBlock.startsWith("@container") ||
        selectorBlock.startsWith("@scope")
      ) {
        if (hasDescendantSvgAbsoluteTrap(body)) return true;
      }
      continue;
    }

    let positionsAbsolutely = false;
    let hasInset = false;
    let hasFullDim = false;
    let hasTop = false;
    let hasLeft = false;
    for (const decl of body.split(";")) {
      const colon = decl.indexOf(":");
      if (colon === -1) continue;
      const prop = decl.slice(0, colon).trim();
      const val = stripImportant(decl.slice(colon + 1));
      if (prop === "position" && (val === "absolute" || val === "fixed")) {
        positionsAbsolutely = true;
      } else if (prop === "inset") {
        hasInset = true;
      } else if (
        (prop === "width" || prop === "height") &&
        (val === "100%" || val === "100vw" || val === "100vh" || val === "100dvh")
      ) {
        hasFullDim = true;
      } else if (prop === "top") {
        hasTop = true;
      } else if (prop === "left") {
        hasLeft = true;
      }
    }
    if (!positionsAbsolutely) continue;
    const isBackdropSized = hasInset || hasFullDim || (hasTop && hasLeft);

    for (const rawSel of selectorBlock.split(",")) {
      const sel = rawSel.trim();
      if (sel === "svg") return true;
      if (isBackdropSized && sel.length >= 4 && sel.endsWith("svg")) {
        const beforeSvg = sel.slice(0, -3);
        if (/\s$/.test(beforeSvg)) {
          const trimmedParent = beforeSvg.trimEnd();
          if (
            trimmedParent.length > 0 &&
            !trimmedParent.endsWith(">") &&
            !trimmedParent.endsWith("+") &&
            !trimmedParent.endsWith("~")
          ) {
            return true;
          }
        }
      }
    }
  }
  return false;
}

const FORBIDDEN_MEDIA_TAGS = new Set(["video", "audio", "source", "object", "embed"]);

const API_RULES: Array<{
  code: string;
  pattern: RegExp;
  message: string;
}> = [
  {
    code: "no-date-now",
    pattern: /(?:^|[^A-Za-z0-9_$])Date\.now\s*\(/,
    message:
      "Non-deterministic API 'Date.now()' is forbidden in Solaris GFX; bind animations to timeline time.",
  },
  {
    code: "no-performance-now",
    pattern: /(?:^|[^A-Za-z0-9_$])performance\.now\s*\(/,
    message:
      "Non-deterministic API 'performance.now()' is forbidden in Solaris GFX; bind animations to timeline time.",
  },
  {
    code: "no-new-date",
    pattern: /(?:^|[^A-Za-z0-9_$])new\s+Date\s*\(/,
    message:
      "Non-deterministic API 'new Date()' is forbidden in Solaris GFX; bind animations to timeline time.",
  },
  {
    code: "no-unseeded-random",
    pattern: /(?:^|[^A-Za-z0-9_$])Math\.random\s*\(/,
    message:
      "Non-deterministic API 'Math.random()' is forbidden in Solaris GFX; use a seeded PRNG.",
  },
  {
    code: "no-set-interval",
    pattern: /(?:^|[^A-Za-z0-9_$])setInterval\s*\(/,
    message:
      "Non-deterministic API 'setInterval()' is forbidden in Solaris GFX; drive motion via __solarisOnSeek or GSAP timelines.",
  },
  {
    code: "no-set-timeout",
    pattern: /(?:^|[^A-Za-z0-9_$])setTimeout\s*\(/,
    message:
      "Non-deterministic API 'setTimeout()' is forbidden in Solaris GFX; drive motion via __solarisOnSeek or GSAP timelines.",
  },
  {
    code: "no-request-animation-frame",
    pattern: /(?:^|[^A-Za-z0-9_$])requestAnimationFrame\s*\(/,
    message:
      "Non-deterministic API 'requestAnimationFrame()' is forbidden in Solaris GFX; drive motion via __solarisOnSeek.",
  },
  {
    code: "no-fetch",
    pattern: /(?:^|[^A-Za-z0-9_$])fetch\s*\(/,
    message:
      "Network API 'fetch()' is forbidden in Solaris GFX; overlays must render offline deterministically.",
  },
  {
    code: "no-xhr",
    pattern: /(?:^|[^A-Za-z0-9_$])XMLHttpRequest(?=$|[^A-Za-z0-9_$])/,
    message:
      "Network API 'XMLHttpRequest' is forbidden in Solaris GFX; overlays must render offline deterministically.",
  },
  {
    code: "no-dynamic-import",
    pattern: /(?:^|[^A-Za-z0-9_$])import\s*\(/,
    message:
      "Dynamic 'import()' is forbidden in Solaris GFX; inline deterministic code only.",
  },
  {
    code: "no-websocket",
    pattern: /(?:^|[^A-Za-z0-9_$])WebSocket(?=$|[^A-Za-z0-9_$])/,
    message:
      "Network API 'WebSocket' is forbidden in Solaris GFX; overlays must render offline deterministically.",
  },
  {
    code: "no-event-source",
    pattern: /(?:^|[^A-Za-z0-9_$])EventSource(?=$|[^A-Za-z0-9_$])/,
    message:
      "Network API 'EventSource' is forbidden in Solaris GFX; overlays must render offline deterministically.",
  },
  {
    code: "no-send-beacon",
    pattern: /(?:^|[^A-Za-z0-9_$])navigator\s*\.\s*sendBeacon(?=$|[^A-Za-z0-9_$])/,
    message:
      "Network API 'navigator.sendBeacon' is forbidden in Solaris GFX; overlays must render offline deterministically.",
  },
  {
    code: "no-window-location",
    pattern: /(?:^|[^A-Za-z0-9_$])window\s*\.\s*location(?=$|[^A-Za-z0-9_$])/,
    message:
      "Navigation API 'window.location' is forbidden in Solaris GFX; overlays must render offline deterministically.",
  },
];

export function lintGfxHtml(html: string): GfxLintIssue[] {
  const issues: GfxLintIssue[] = [];
  const { tags, scriptSources, styleSources } = scanHtml(html);

  let hasExtScript = false;
  let hasExtResource = false;
  let hasEmbeddedMedia = false;
  let hasMetaRefresh = false;

  for (const tag of tags) {
    if (tag.name === "script" && tag.attrs.some(([k]) => k === "src")) {
      hasExtScript = true;
    } else if (
      tag.name === "img" &&
      tag.attrs.some(
        ([k, v]) =>
          (k === "src" && isExternalUrl(v)) || (k === "srcset" && hasExternalSrcsetUrl(v)),
      )
    ) {
      hasExtResource = true;
    } else if (
      tag.name === "link" &&
      tag.attrs.some(([k, v]) => k === "href" && isExternalUrl(v))
    ) {
      hasExtResource = true;
    } else if (
      tag.name === "iframe" &&
      tag.attrs.some(([k, v]) => k === "src" && isExternalUrl(v))
    ) {
      hasExtResource = true;
    } else if (
      tag.name === "image" &&
      tag.attrs.some(([k, v]) => (k === "href" || k === "xlink:href") && isExternalUrl(v))
    ) {
      hasExtResource = true;
    } else if (FORBIDDEN_MEDIA_TAGS.has(tag.name)) {
      hasEmbeddedMedia = true;
    } else if (
      tag.name === "meta" &&
      tag.attrs.some(([k, v]) => k === "http-equiv" && v.trim().toLowerCase() === "refresh")
    ) {
      hasMetaRefresh = true;
    }
  }

  let hasCssImport = false;
  let hasExtCssUrl = false;
  let hasDescendantSvgAbs = false;
  for (const rawCss of styleSources) {
    const css = stripCssComments(rawCss);
    if (!hasCssImport && CSS_IMPORT_RE.test(css)) {
      hasCssImport = true;
    }
    if (!hasExtCssUrl) {
      for (const url of extractCssUrls(css)) {
        if (isExternalUrl(url)) {
          hasExtCssUrl = true;
          break;
        }
      }
    }
    if (!hasDescendantSvgAbs && hasDescendantSvgAbsoluteTrap(css)) {
      hasDescendantSvgAbs = true;
    }
  }

  if (hasExtScript) {
    issues.push({
      code: "no-external-script",
      severity: "error",
      message:
        "External <script src=...> tags are forbidden in Solaris GFX; use inline deterministic CSS or runtime bindings.",
    });
  }

  if (hasExtResource) {
    issues.push({
      code: "no-external-resource",
      severity: "error",
      message:
        "External URLs in <img>, <image>, <link>, or <iframe> tags are forbidden in Solaris GFX; inline data: URIs or CSS tokens instead.",
    });
  }

  if (hasEmbeddedMedia) {
    issues.push({
      code: "no-embedded-media",
      severity: "error",
      message:
        "Embedded media/plugin tags (<video>, <audio>, <source>, <object>, <embed>) are forbidden in Solaris GFX; use deterministic HTML/CSS/SVG.",
    });
  }

  if (hasMetaRefresh) {
    issues.push({
      code: "no-meta-refresh",
      severity: "error",
      message:
        "<meta http-equiv=\"refresh\"> is forbidden in Solaris GFX; overlays must render offline deterministically.",
    });
  }

  if (hasCssImport) {
    issues.push({
      code: "no-css-import",
      severity: "error",
      message:
        "CSS '@import' is forbidden in Solaris GFX; inline styles and brand tokens only.",
    });
  }

  if (hasExtCssUrl) {
    issues.push({
      code: "no-external-css-url",
      severity: "error",
      message:
        "External URLs in CSS url(...) are forbidden in Solaris GFX; use data: URIs or fragment (#) references instead.",
    });
  }

  if (scriptSources.length > 0) {
    const combinedScripts = scriptSources.join("\n");
    for (const rule of API_RULES) {
      if (rule.pattern.test(combinedScripts)) {
        issues.push({
          code: rule.code,
          severity: "error",
          message: rule.message,
        });
      }
    }
  }

  if (!html.includes("data-start") || !html.includes("data-duration")) {
    issues.push({
      code: "missing-timing-attrs",
      severity: "warning",
      message:
        "Root element should include data-start and data-duration attributes for deterministic seeking.",
    });
  }

  if (!html.includes("--brand-")) {
    issues.push({
      code: "missing-brand-tokens",
      severity: "warning",
      message:
        "Consider using Brand Playbook CSS variables (e.g. var(--brand-accent), var(--brand-ink)).",
    });
  }

  const svgTagCount = tags.filter((t) => t.name === "svg").length;
  const hasSvgTag = svgTagCount > 0;
  if (svgTagCount >= 2 && hasDescendantSvgAbs) {
    issues.push({
      code: "descendant-svg-absolute",
      severity: "warning",
      message:
        "Descendant CSS selector targeting 'svg' with position:absolute/fixed can capture nested icon <svg> elements; use a direct-child selector (e.g. '.funnel > svg') or an explicit class on the background <svg>.",
    });
  }
  if (
    hasSvgTag &&
    scriptSources.some(
      (js) =>
        /(?:^|[^A-Za-z0-9_$])rotation(?=$|[^A-Za-z0-9_$])/.test(js) &&
        !js.includes("svgOrigin") &&
        !js.includes("transformOrigin"),
    )
  ) {
    issues.push({
      code: "svg-rotation-origin",
      severity: "warning",
      message:
        'SVG rotation animation without svgOrigin or transformOrigin may orbit off-center; set svgOrigin: "<cx> <cy>" matching the viewBox center.',
    });
  }

  return issues;
}
