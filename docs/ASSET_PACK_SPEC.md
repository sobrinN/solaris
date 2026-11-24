# Solaris Community Code-Asset Pack Specification (`solaris.asset-pack/v1`)

Solaris motion graphics (`SavedCodeAsset`) are zero-build HTML/CSS/GSAP snippets parameterized with JSON Schema props and Brand Playbook CSS custom properties (`--brand-*`). Packs can be exported and imported via HTTP, MCP, or the Studio UI.

---

## 1. Manifest Format

```json
{
  "schema_version": "solaris.asset-pack/v1",
  "pack_id": "solaris-workspace-pack",
  "name": "Solaris Workspace Asset Pack",
  "version": "1.0.0",
  "author": "Solaris Community",
  "license": "Apache-2.0",
  "description": "Reusable Solaris HTML/GSAP motion graphics asset pack",
  "assets": [
    {
      "id": "stat-card-solar",
      "name": "Stat Card",
      "category": "cards",
      "tags": ["stat", "metric"],
      "props_schema": {
        "type": "object",
        "properties": {
          "value": { "type": "string" },
          "label": { "type": "string" }
        }
      },
      "default_props": { "value": "10 HRS", "label": "SAVED EVERY WEEK" },
      "html_source": "<div class=\"clip\" data-start=\"0\" data-duration=\"3.0\" style=\"color:var(--brand-ink,#F7F5EF);\">{{value}}</div>",
      "default_duration_sec": 3.0,
      "created_at": "2026-11-23T00:00:00Z"
    }
  ]
}
```

---

## 2. Validation & Deterministic Lint Contract

Every imported pack is validated by `validate_asset_pack_manifest` (`crates/core-ir/src/registry.rs`) and `lint_gfx_html` (`crates/agent-runtime/src/lint.rs`):

1. **Schema & IDs**: `schema_version` must be `"solaris.asset-pack/v1"`; `pack_id` and every `asset.id` must be 1–64 ASCII `[a-zA-Z0-9_-]` with no duplicates (1–50 assets per pack).
2. **Offline & Sandboxed Resources**: External `<script src=...>`, external URLs in `<img>` (`src`/`srcset`), `<link href=...>`, `<iframe src=...>`, SVG `<image href=...>` / `xlink:href=...`, CSS `@import`, non-`data:` CSS `url(...)`, `<meta http-equiv="refresh">`, and embedded media/plugin tags (`<video>`, `<audio>`, `<source>`, `<object>`, `<embed>`) are rejected. Inline `<style>...</style>` blocks, `#fragment` refs, and `data:` image URIs are allowed.
3. **Deterministic & Offline Scripts Only**: Inside `<script>` blocks and inline `on*=` handlers, `Date.now(`, `performance.now(`, `new Date(`, `Math.random(`, `setInterval(`, `setTimeout(`, `requestAnimationFrame(`, `fetch(`, `XMLHttpRequest`, `import(`, `WebSocket`, `EventSource`, `navigator.sendBeacon`, and `window.location` are rejected.
4. **Root Attributes & Brand Tokens**: Roots should declare `data-start` and `data-duration`, and style colors/fonts with `--brand-bg`, `--brand-surface`, `--brand-ink`, `--brand-accent`, `--brand-accent-secondary`, `--brand-font-display`, and `--brand-font-body`.

---

## 3. Endpoints & MCP Resources

- **Export**: `GET /api/v1/assets/export` (optional `?ids=id1,id2`)
- **Import**: `POST /api/v1/assets/import`
- **MCP Resource**: `solaris://assets/pack`
