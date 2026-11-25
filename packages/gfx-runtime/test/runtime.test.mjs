import test from "node:test";
import assert from "node:assert/strict";
import {
  brandTokensToCss,
  buildSandboxedGfxDoc,
  escapeHtml,
  extractPropFields,
  lintGfxHtml,
} from "../dist/index.js";

test("escapes HTML in interpolated props, resolves nested keys, and sanitizes brand CSS tokens", () => {
  assert.equal(escapeHtml("<script>alert(1)</script>"), "&lt;script&gt;alert(1)&lt;/script&gt;");

  const tokens = {
    bg: "#080404 /* comment */ ; } body { display: none; }",
    surface: "#1A0E0E//*/*comment*/*/",
    ink: "#F7F5EF",
    accent: "#F05A00",
    accent_secondary: "#F7B733",
    font_display: 'Sora"}; </style><script>',
    font_body: "Sora",
  };
  const css = brandTokensToCss(tokens);
  assert.match(css, /--brand-accent: #F05A00/);
  assert.doesNotMatch(css, /<\/style>/);
  assert.doesNotMatch(css, /--brand-bg: #080404;/);
  assert.doesNotMatch(css, /\/\*|\*\//);

  const doc = buildSandboxedGfxDoc({
    htmlSource: "<div>{{headline}} · {{meta.author}}</div>",
    props: { headline: "<b>Bold</b>", meta: { author: "@solaris" } },
    brandTokens: tokens,
    durationSec: 3.0,
    initialTimeSec: 1.2,
    fontCss: "@font-face { font-family: 'Test'; } </sty</stylele><script>alert(1)</script>",
  });
  assert.match(doc, /Content-Security-Policy/);
  assert.match(doc, /default-src 'none'/);
  assert.match(doc, /color-scheme:\s*dark/);
  assert.doesNotMatch(doc, /<\/style><script>alert\(1\)/);
  assert.match(doc, /&lt;b&gt;Bold&lt;\/b&gt; · @solaris/);
  assert.match(doc, /window\.__solarisSeek/);
  assert.match(doc, /solaris:seek/);
  assert.match(doc, /solaris:gfx-ready/);
});

test("lintGfxHtml matches Rust linter parity without false positives", () => {
  const clean =
    '<style>@keyframes pop { from { opacity: 0; } to { opacity: 1; } }</style><div data-start="0" data-duration="2" style="color:var(--brand-ink)"><img src="data:image/png;base64,iVBORw0KGgo=" /><script>window.__solarisOnSeek = (t) => t;</script></div>';
  assert.deepEqual(lintGfxHtml(clean), []);

  const issues = lintGfxHtml(
    '<script src="https://cdn.example.com/a.js"></script><img src="https://evil.test/x.png"><script>Date.now (); performance.now(); Math.random(); setTimeout(() => {}, 10); setInterval(() => {}, 10); requestAnimationFrame(() => {}); fetch("https://a.b"); new XMLHttpRequest(); import("https://a.b/m.js");</script>',
  );
  const errors = issues.filter((i) => i.severity === "error");
  assert.equal(errors.length, 11);

  const fields = extractPropFields(
    {
      properties: {
        value: { type: "string" },
        count: { type: "number" },
        visible: { type: "boolean" },
        accent_color: { type: "string", format: "color", default: "not-hex" },
        subtitle: { type: "string", default: "Solaris Default" },
        metadata: { type: "object", default: { rank: 1 } },
      },
    },
    { value: "10 HRS", count: 42, visible: true },
  );
  assert.equal(fields.length, 6);
  assert.equal(fields[1].kind, "number");
  assert.equal(fields[2].kind, "boolean");
  assert.equal(fields[3].kind, "color");
  assert.equal(fields[3].value, "#F05A00");
  assert.equal(fields[4].value, "Solaris Default");
  assert.equal(fields[5].value, '{"rank":1}');
});

test("lintGfxHtml blocks CSS imports/URLs, new Date, WebSocket, meta refresh, embedded media, srcset, and SVG external image while allowing safe HTML/data URIs", () => {
  // Plain HTML text & comments mentioning fetch() must NOT trigger false positives
  const safeText =
    '<style>@keyframes slide { from { transform: translateY(8px); } to { transform: translateY(0); } } .badge { background: url("data:image/png;base64,iVBORw0KGgo="); clip-path: url(#mask); }</style>' +
    '<!-- fetch("https://example.com") new Date() -->' +
    '<div data-start="0" data-duration="2" style="color:var(--brand-ink)">How fetch() works<img src="data:image/png;base64,iVBORw0KGgo=" srcset="data:image/png;base64,iVBORw0KGgo= 1x" /><svg><image href="data:image/png;base64,iVBORw0KGgo=" /></svg><script>const $fetch = (x) => x; $fetch(1);</script></div>';
  assert.deepEqual(lintGfxHtml(safeText), []);

  const cssImport = lintGfxHtml(
    '<style>@import url("https://fonts.example.com/font.css");</style><div data-start="0" data-duration="2" style="color:var(--brand-ink)">Hi</div>',
  );
  assert.ok(cssImport.some((i) => i.severity === "error" && i.code === "no-css-import"));

  const cssExternalUrl = lintGfxHtml(
    '<div data-start="0" data-duration="2" style="color:var(--brand-ink);background:url(https://evil.test/bg.png)">Hi</div>',
  );
  assert.ok(cssExternalUrl.some((i) => i.severity === "error" && i.code === "no-external-css-url"));

  const cssFileScheme = lintGfxHtml(
    '<style>.a { background: url("file:///etc/passwd"); }</style><div data-start="0" data-duration="2" style="color:var(--brand-ink)">Hi</div>',
  );
  assert.ok(cssFileScheme.some((i) => i.severity === "error" && i.code === "no-external-css-url"));

  const jsApis = lintGfxHtml(
    '<div data-start="0" data-duration="2" style="color:var(--brand-ink)" onclick="window.location = \'https://evil.test\'"><script>const d = new Date(); const ws = new WebSocket("wss://evil.test"); const es = new EventSource("/sse"); navigator.sendBeacon("/log");</script></div>',
  );
  const jsErrorCodes = new Set(jsApis.filter((i) => i.severity === "error").map((i) => i.code));
  assert.ok(jsErrorCodes.has("no-new-date"));
  assert.ok(jsErrorCodes.has("no-websocket"));
  assert.ok(jsErrorCodes.has("no-event-source"));
  assert.ok(jsErrorCodes.has("no-send-beacon"));
  assert.ok(jsErrorCodes.has("no-window-location"));

  const metaRefresh = lintGfxHtml(
    '<meta http-equiv="refresh" content="0;url=https://evil.test" /><div data-start="0" data-duration="2" style="color:var(--brand-ink)">Hi</div>',
  );
  assert.ok(metaRefresh.some((i) => i.severity === "error" && i.code === "no-meta-refresh"));

  const videoTag = lintGfxHtml(
    '<video src="https://evil.test/v.mp4"></video><div data-start="0" data-duration="2" style="color:var(--brand-ink)">Hi</div>',
  );
  assert.ok(videoTag.some((i) => i.severity === "error" && i.code === "no-embedded-media"));

  const srcsetTag = lintGfxHtml(
    '<img srcset="https://evil.test/1x.png 1x, https://evil.test/2x.png 2x" /><div data-start="0" data-duration="2" style="color:var(--brand-ink)">Hi</div>',
  );
  assert.ok(srcsetTag.some((i) => i.severity === "error" && i.code === "no-external-resource"));

  const svgExternalImage = lintGfxHtml(
    '<svg><image xlink:href="https://evil.test/img.png" /></svg><div data-start="0" data-duration="2" style="color:var(--brand-ink)">Hi</div>',
  );
  assert.ok(
    svgExternalImage.some((i) => i.severity === "error" && i.code === "no-external-resource"),
  );
});

test("buildSandboxedGfxDoc sanitizes NaN timings, re-seeks on DOMContentLoaded/load, and validates ev.source", () => {
  const tokens = {
    bg: "#080404",
    surface: "#1A0E0E",
    ink: "#F7F5EF",
    accent: "#F05A00",
    accent_secondary: "#F7B733",
    font_display: "Space Grotesk",
    font_body: "Inter",
  };
  const doc = buildSandboxedGfxDoc({
    htmlSource: "<div>{{title}}</div>",
    props: { title: "Solaris" },
    brandTokens: tokens,
    durationSec: Number.NaN,
    initialTimeSec: Number.NaN,
  });

  assert.match(doc, /const DURATION = 3;/);
  assert.match(doc, /let lastSeekTime = 0;/);
  assert.doesNotMatch(doc, /const DURATION = NaN/);
  assert.match(doc, /DOMContentLoaded/);
  assert.match(doc, /window\.addEventListener\("load"/);
  assert.match(doc, /ev\.source !== window\.parent/);
});

test("brandTokensToCss includes Sora fallback in display and body font stacks", () => {
  const css = brandTokensToCss({
    bg: "#080404",
    surface: "#1A0E0E",
    ink: "#F7F5EF",
    accent: "#F05A00",
    accent_secondary: "#F7B733",
    font_display: "Custom Display",
    font_body: "Custom Body",
  });

  assert.match(css, /--brand-font-display: "Custom Display", "Sora", sans-serif;/);
  assert.match(css, /--brand-font-body: "Custom Body", "Sora", sans-serif;/);
});

test("extractPropFields handles null props, null values falling back to defaults, and boolean string parsing", () => {
  const schema = {
    properties: {
      title: { type: "string", default: "Default Title" },
      flagFalse: { type: "boolean", default: true },
      flagZero: { type: "boolean", default: true },
      flagEmpty: { type: "boolean", default: true },
      flagTrue: { type: "boolean", default: false },
      flagNull: { type: "boolean", default: true },
    },
  };

  const fromNullProps = extractPropFields(schema, null);
  assert.equal(fromNullProps.length, 6);
  assert.equal(fromNullProps.find((f) => f.key === "title")?.value, "Default Title");
  assert.equal(fromNullProps.find((f) => f.key === "flagNull")?.value, true);

  const parsed = extractPropFields(schema, {
    title: null,
    flagFalse: "false",
    flagZero: "0",
    flagEmpty: "",
    flagTrue: "true",
    flagNull: null,
  });
  assert.equal(parsed.find((f) => f.key === "title")?.value, "Default Title");
  assert.equal(parsed.find((f) => f.key === "flagFalse")?.value, false);
  assert.equal(parsed.find((f) => f.key === "flagZero")?.value, false);
  assert.equal(parsed.find((f) => f.key === "flagEmpty")?.value, false);
  assert.equal(parsed.find((f) => f.key === "flagTrue")?.value, true);
  assert.equal(parsed.find((f) => f.key === "flagNull")?.value, true);
});
