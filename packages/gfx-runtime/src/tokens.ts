export interface GfxBrandTokens {
  bg: string;
  surface: string;
  ink: string;
  accent: string;
  accent_secondary: string;
  font_display: string;
  font_body: string;
}

function sanitizeCssValue(raw: string): string {
  let out = String(raw ?? "").replace(/[;{}<>\\\r\n]/g, "");
  let prev: string;
  do {
    prev = out;
    out = out.replace(/\/\*|\*\//g, "");
  } while (out !== prev);
  return out.trim();
}

export function brandTokensToCss(tokens: GfxBrandTokens): string {
  const fontDisplay = sanitizeCssValue(tokens.font_display).replace(/["']/g, "");
  const fontBody = sanitizeCssValue(tokens.font_body).replace(/["']/g, "");
  return `:root {
  --brand-bg: ${sanitizeCssValue(tokens.bg)};
  --brand-surface: ${sanitizeCssValue(tokens.surface)};
  --brand-ink: ${sanitizeCssValue(tokens.ink)};
  --brand-accent: ${sanitizeCssValue(tokens.accent)};
  --brand-accent-secondary: ${sanitizeCssValue(tokens.accent_secondary)};
  --brand-font-display: "${fontDisplay}", "Sora", sans-serif;
  --brand-font-body: "${fontBody}", "Sora", sans-serif;
}`;
}
