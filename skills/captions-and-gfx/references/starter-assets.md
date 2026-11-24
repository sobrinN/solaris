# Solaris Starter GFX Asset Catalog

Always call `assets.search` first to check both these 5 built-in assets and any workspace-saved custom assets before authoring new `inline_html`.

| `asset_id` | Category | Props Schema & Defaults | Default Dur | Recommended `G1` Placement (`timeline.set_transform`) | Best Use |
| --- | --- | --- | --- | --- | --- |
| `chapter-title-solar` | `titles` | `eyebrow`: `"THE LIST"`<br/>`headline`: `"3 AI TOOLS"` | `2.5s` | `y: -0.22`, `scale: 0.95` (9:16) or `y: -0.18` (16:9) | Opening 0–2.5s hook title or section chapter divider. |
| `stat-card-solar` | `cards` | `value`: `"10 HRS"`<br/>`label`: `"SAVED EVERY WEEK"` | `3.0s` | `y: -0.22`, `scale: 0.92` | Landing a real sourced statistic or quantified benefit. |
| `comment-bubble-solar` | `social` | `author`: `"@creator"`<br/>`comment`: `"the voice clone one is insane"` | `2.8s` | `y: -0.24`, `scale: 0.92` | Social proof, audience question hook, or closing CTA prompt. |
| `lower-third-solar` | `titles` | `title`: `"Speaker Name"`<br/>`subtitle`: `"Founder & Host"` | `3.2s` | `y: -0.24` (when `T1` captions are at `y: 0.20`) or `y: 0.24` (if no captions active) | Speaker attribution, tool name callout, or source citation. |
| `progress-bar-solar` | `overlays` | `label`: `"STEP 01 / 03"`<br/>`percent`: `65` (`number`, `0–100`) | `3.0s` | `y: -0.30`, `scale: 0.90` | Multi-step listicles, tutorials, and retention trackers. |

## Usage Pattern

```json
// Step 1: Place asset on G1
{
  "tool": "gfx.place",
  "arguments": {
    "project_id": "<project_id>",
    "clip_id": "gfx-stat-1",
    "label": "10 HRS Saved",
    "start_sec": 4.2,
    "duration_sec": 3.0,
    "asset_id": "stat-card-solar",
    "props": { "value": "10 HRS", "label": "SAVED EVERY WEEK" }
  }
}

// Step 2: Offset clear of T1 captions (y: 0.20)
{
  "tool": "timeline.set_transform",
  "arguments": {
    "project_id": "<project_id>",
    "track_id": "g1",
    "clip_id": "gfx-stat-1",
    "x": 0.0,
    "y": -0.22,
    "scale": 0.92
  }
}
```
