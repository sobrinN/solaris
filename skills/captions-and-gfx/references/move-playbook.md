# Solaris Programmatic GFX Move Playbook (10-Category Pattern Library)

Use this playbook whenever choosing or authoring a `G1` motion-graphic move—either as a **transparent overlay** on top of a talking-head video (`V1`) or as a **full-canvas programmatic scene** (`100vw × 100vh`) in a faceless explainer or zero-asset video.

## Placement Rules Before Building

1. **One Move Per Idea Worth Seeing**: Place a graphic only when the speaker states a concrete number, comparison, step, tool, quote, or turning point. Never stack two floating overlays within `4.0s` of each other.
2. **Instrument, Not a Sentence**: To show "fast", animate a counter or stopwatch stopping; "growth", a sparkline or bar race; "comparison", a 2×2 matrix or pros/cons split; "proof", a receipt, quote card, or KPI tile.
3. **Overlay vs Full-Canvas Cutaway**:
   - **Floating Overlay (over `V1` talking head)**: Transparent background (no full-screen backdrop/vignette), give cards enough opaque/frosted body (`rgba(26,14,14,0.88)` + `1px` rim) to read over any footage, enter from empty at `t=0`, exit cleanly in the last `0.30–0.40s`, and call `timeline.set_transform(track_id="g1", y=-0.22)` so it stays clear of the face and `T1` captions (`y: 0.20`).
   - **Full-Canvas Programmatic Scene (zero-asset or B-roll cutaway)**: Fill `100vw × 100vh` with layered ambient glow + subtle grid/grain, keep `Transform2D` at `(0, 0)`, and hold core visuals visible to the scene cut (no fade-to-black before exit). On `9:16`, keep foreground inside `6vh..62vh` when `T1` captions are active, or distribute evenly across `8vh..84vh` when no `T1` captions exist (never leaving a bottom-half black void).

---

## 1. The 10-Category Spoken-Cue → Visual-Move Taxonomy

A **curated 10-category taxonomy distilled from 260+ motion moves**. Match what the script or speaker says to the exact visual archetype:

> **Overlay vs Explainer Quick-Pick**
> - **Talking-Head Overlay Stars** (transparent alpha over `V1`): `notify-stack`, `chapter-tag`, `listicle-number`, `follower-ticker-chip`, `verdict-card`, `strike-correct`, `headline-highlight`, `captions-burn`.
> - **Full-Canvas Explainer Stars** (`100vw × 100vh` 3-zone grid): `kpi-tiles`, `bar-race`, `waterfall-chart`, `dashboard-assembly`, `exploded-ui`, `spotlight-dim`, `tool-call-trace`, `terminal-run`, `code-diff`.

### A. Data & Proof (Numbers, Growth, Rankings, Comparisons)
| Spoken Cue / Script Beat | Visual Move Archetype | Core Solaris SVG/GSAP Mechanic |
| --- | --- | --- |
| Reels off 2–4 results ("revenue up 32%, users up 18%") | **`kpi-tiles`** | 2×2 (9:16) or 1×4 (16:9) glass tiles fly in from 3D depth → numbers count (`tabular-nums`) while SVG sparklines draw → hero tile lifts with accent rim & sheen |
| Compares options by one number ("X is 3× faster") | **`bar-race`** | Horizontal bars grow with `expo.out`, leader overtakes and stamps with accent halo |
| Growth or trend over time | **`line-reveal`** / **`line-cross`** | Smooth cubic SVG curve draws via `strokeDashoffset`, glowing dot rides tip, crossing point triggers pulse ring + value pop |
| Conversion drop-off across steps | **`funnel-stages`** | Tapered stepped bands reveal top-to-bottom with `%` counts and conversion deltas |
| Breaks total into parts / where time/money goes | **`stacked-bars`** / **`treemap`** / **`bubble-pack`** | Proportional blocks/circles scale in (`back.out(1.4)`), dominant share highlights in `--brand-accent` |
| Adds several numbers into one total | **`bracket-sum`** / **`waterfall-chart`** | Line items stagger in → SVG curly bracket draws → final sum counts & stamps |
| Ranks top 3 or overtakes #1 | **`podium-rank`** / **`leaderboard`** | Rows slide/reorder by `y` transform; winner row elevates with rim glow |
| "High impact, low effort" / beats trade-offs | **`matrix-2x2`** / **`scatter-quadrant`** / **`radar-chart`** | Orthogonal axes draw → competitor dots pop → hero dot lands in target quadrant with halo |
| "Only 3% do this" / "37 out of 100" | **`distribution-curve`** / **`unit-dots`** | Bell curve tail fills in accent, or 10×10 dot grid lights up exact count sequentially |
| Streak, duration, or overload | **`day-counter`** / **`contribution-heatmap`** / **`gauge-needle`** | Flip/slot day count, GitHub-style activity grid wave, or SVG arc needle sweeping into red/accent zone |
| Retention or multi-channel comparison | **`cohort-grid`** / **`sparkline-list`** / **`slope-chart`** / **`tier-pyramid`** | Color-coded retention cells, inline mini-sparklines, before→after slope lines, or stacked pyramid tiers |

### B. Explain & Frameworks (Breaking Ideas Into Parts)
| Spoken Cue / Script Beat | Visual Move Archetype | Core Solaris SVG/GSAP Mechanic |
| --- | --- | --- |
| Sums up what a product/system includes | **`bento-reveal`** | Asymmetric 3–5 tile bento grid springs in (`0.06s` stagger), each tile runs a tiny live SVG micro-viz |
| Breaks concept into N parts / architecture | **`explainer-build`** / **`exploded-ui`** | Single card splits/expands into labeled layers with orthogonal SVG leader lines |
| Lists N steps, names them as one system | **`step-bracket-label`** / **`timeline-track`** | Nodes light up sequentially along an SVG rail, bracket groups them under the unified label |
| Weighs trade-offs or gives a verdict | **`pros-cons`** / **`verdict-card`** | Two-column split (semantic green checks vs red crosses), or bold verdict card stamped with `APPROVED` / `SKIP` |
| "What you see is only the tip" | **`iceberg-reveal`** | Waterline bisects frame; surface label sits above, camera/rig pans down to reveal hidden layers below |
| "One thing becomes many" / connected parts | **`mind-map`** / **`chain-links`** | Center node bursts SVG bezier branches to child pills, or interlocking chain links snap together |
| "Everyone does X... except one" / holds step back | **`odd-one-out-highlight`** / **`locked-list-reveal`** | Grid of muted cards dims while 1 hero card lifts; or numbered list shows blurred/locked row that unlocks with stamp |
| Calls out a detail / focuses attention | **`spotlight-dim`** / **`magnifier-loupe`** / **`label-box-callout`** | Darkened backdrop dims surroundings while circular mask or loupe ring zooms/frames the target detail |
| Adjusts one lever/setting | **`rate-dial-knob`** | Rotary SVG dial rotates smoothly while linked output metric counts in real time |

### C. AI & Automation (Agents, Prompts, Pipelines)
| Spoken Cue / Script Beat | Visual Move Archetype | Core Solaris SVG/GSAP Mechanic |
| --- | --- | --- |
| Multi-stage workflow ("brief → research → edit") | **`agent-pipeline`** / **`tool-map`** | Connected glass nodes along glowing SVG rail; active pulse travels node-to-node lighting each stage |
| "Just type this prompt..." / AI streams answer | **`ask-bar`** / **`ai-stream`** / **`prompt-template-copy`** | Command bar types query (`28 cps`), cursor clicks Send, response lines stream in with subtle caret |
| Multiple agents working in parallel / hub | **`agents-board`** / **`agent-hub`** | Central core with radial SVG data paths firing into 3–5 concurrent status cards |
| "It cites sources / checks before acting" | **`cited-answer`** / **`agent-approval`** / **`tool-call-trace`** | Answer chips pop inline `[1] [2]`, or trace log steps tick green sequentially ending in Approve button press |
| Context limits / model switching / automations | **`context-meter`** / **`model-picker`** / **`automation-rule`** / **`connector-toggle`** | Segmented fill bar approaching limit, dropdown selection click, `IF → THEN` rule bridge, or switch toggles snapping green |

### D. Dev & Engineering (Code, Terminal, Shipping)
| Spoken Cue / Script Beat | Visual Move Archetype | Core Solaris SVG/GSAP Mechanic |
| --- | --- | --- |
| "One command to install/deploy" | **`terminal-run`** | Dark monospace window, command types out, progress bar fills, semantic green `✓ Ready` line lands |
| "AI rewrote the function" / fixed a bug | **`code-diff`** / **`error-to-fix`** / **`review-suggestion`** | Red `-` lines strike/collapse while green `+` lines wipe in; red error badge flips to green `Tests Passed` |
| Speedup / API call / deploy / PR merge | **`network-waterfall`** / **`api-call`** / **`deploy-live`** / **`pr-merge`** | Waterfall timing bars shrink dramatically (`4.8s → 0.3s`), JSON request→`200 OK` badge, or purple→green merged pill |
| Branches, logs, status, SQL, gradual rollout | **`git-graph`** / **`log-filter`** / **`status-page`** / **`sql-query`** / **`rollout-slider`** | SVG commit rail merging, log rows filtering to 1 highlighted match, uptime bars turning green, or `1% → 100%` slider |

### E. Social Proof & Creator Economy
| Spoken Cue / Script Beat | Visual Move Archetype | Core Solaris SVG/GSAP Mechanic |
| --- | --- | --- |
| Retells a text/DM conversation | **`chat-thread`** / **`dm-inbox`** / **`slack-message`** | Staggered left/right message bubbles pop in (`back.out(1.8)`) with realistic timestamps |
| Reads viewer comment / viral post / CTA | **`comment-card`** / **`comment-keyword`** / **`post-card`** / **`tweet-thread`** | Author avatar circle + quote + heart/reply counters; keyword pill types and highlights |
| Audience poll, ratings, testimonials | **`community-poll`** / **`review-stars`** / **`testimonial-wall`** | Animated poll bars racing to `%`, 5 SVG stars filling sequentially, or staggered 2×2 quote cards |
| Growth / retention / CTR / listicle number | **`reel-insights`** / **`retention-graph`** / **`thumb-ctr-row`** / **`listicle-number`** / **`chapter-tag`** | Metric tiles + retention curve highlight, giant ordinal badge (`03`), or scrub-bar chapter indicator |
| Subscribe / Follow / Lead magnet | **`follow-card`** / **`subscribe-button`** / **`lead-magnet-cover`** | Cursor clicks Follow/Subscribe → button flips to `Following ✓` with subtle particle ring |

### F. Business, Productivity, Editorial Paper & System UI
| Category | High-Impact Archetypes | Core Visual Mechanic |
| --- | --- | --- |
| **Business & Commerce** | `pricing-toggle`, `price-tag` (strike old → reveal new), `invoice-paid`, `payment-success`, `crm-deal-card`, `money-flow-path`, `coupon-code-badge`, `order-tracker`, `waitlist-spot` | Monthly→Yearly toggle flip, semantic green `PAID` stamp, animated SVG flow arrows between wallet/bank/revenue nodes |
| **Productivity & Workflow** | `checklist`, `step-tracker`, `kanban-move`, `calendar-week`, `inbox-zero`, `spotlight-search`, `spreadsheet-fill`, `timer-stopwatch`, `meeting-notes` | Sequential green SVG checkmarks (`strokeDashoffset`), card sliding column-to-column, crisp `⌘K` search bar typing |
| **Editorial & Paper** | `headline-highlight`, `strike-correct`, `stamp-approve`, `receipt-print`, `report-card`, `sticky-note`, `index-card-flip`, `blueprint`, `quote-card`, `paper-tear` | Yellow/accent highlighter wipe behind text, red diagonal strike + replacement word slam, thermal receipt scrolling up, 3D `rotateY(180deg)` card flip |
| **OS & System UI** | `notify-stack`, `keycap-shortcut`, `control-toggles`, `software-update`, `storage-bar`, `battery-charge`, `window-tile`, `tabs-multiply`, `permission-dialog`, `location-pin-chip` | 3 stacked frosted notifications popping in, tactile 3D keycap depression (`y: +4px` + shadow shrink), switches flipping green |

---

## 2. Production-Ready Solaris HTML/CSS/GSAP Blueprints

Every blueprint below is **100% Solaris-native**, passes `lint_gfx_html` with zero errors and zero `descendant_svg_trap` warnings, uses `--brand-*` tokens, `window.gsap`, `window.__timelines["main"]`, responsive units, zero banned slide chrome, and the **3-Zone Full-Stage CSS Grid** on full-canvas scenes (switching to `height: 56vh; padding: 6vh 5vw` when `T1` captions are active). Adapt copy, numbers (only sourced!), and layout to the active scene.

### Blueprint 1: 3D Glass KPI Tiles (Sparklines + Count-Up + Hero Lift)
Best for: proving 2–4 real sourced metrics on the drop or payoff beat.

```html
<div class="clip" data-start="0" data-duration="4.0" style="position:fixed;inset:0;width:100vw;height:100vh;overflow:hidden;background:var(--brand-bg,#080404);font-family:var(--brand-font-body,Sora),sans-serif;color:var(--brand-ink,#F7F5EF);">
  <style>
    .amb{position:absolute;inset:-20%;pointer-events:none}
    .amb i{position:absolute;border-radius:50%;filter:blur(90px);opacity:.38;background:radial-gradient(circle,var(--brand-accent,#F05A00) 0%,transparent 68%)}
    /* Use height: 56vh; padding: 6vh 5vw when T1 captions are active */
    .stage{position:relative;width:100%;height:100vh;display:grid;grid-template-rows:auto 1fr auto;row-gap:2vh;align-items:center;padding:7vh 5vw 9vh;box-sizing:border-box;perspective:1600px}
    .eyebrow{font-size:clamp(12px,2.4vmin,18px);font-weight:700;letter-spacing:.18em;text-transform:uppercase;color:var(--brand-accent-secondary,#F7B733)}
    .grid{display:grid;grid-template-columns:repeat(2,1fr);gap:2.4vmin;transform-style:preserve-3d}
    .tile{position:relative;padding:2.6vmin;border-radius:22px;overflow:hidden;
      background:linear-gradient(155deg,rgba(255,255,255,.15),rgba(255,255,255,.04));
      -webkit-backdrop-filter:blur(20px) saturate(150%);backdrop-filter:blur(20px) saturate(150%);
      box-shadow:inset 0 1px 0 rgba(255,255,255,.42),inset 0 0 0 1px rgba(255,255,255,.12),0 28px 60px -24px rgba(0,0,0,.65)}
    .tile::after{content:"";position:absolute;inset:0;pointer-events:none;mix-blend-mode:screen;
      background:linear-gradient(110deg,transparent calc(var(--sheen,-45%) - 18%),rgba(255,255,255,.28) var(--sheen,-45%),transparent calc(var(--sheen,-45%) + 18%))}
    .hero-ring{position:absolute;inset:0;border-radius:inherit;pointer-events:none;opacity:0;
      box-shadow:inset 0 1px 0 rgba(255,255,255,.55),inset 0 0 0 2px color-mix(in srgb,var(--brand-accent,#F05A00) 75%,white),0 28px 70px -20px color-mix(in srgb,var(--brand-accent,#F05A00) 45%,transparent)}
    .top{display:flex;justify-content:space-between;align-items:center;margin-bottom:1.2vmin}
    .lbl{font-size:clamp(12px,2.5vmin,18px);font-weight:600;opacity:.72}
    .delta{font-size:clamp(11px,2.1vmin,15px);font-weight:700;padding:.3em .65em;border-radius:999px;color:#2BD67B;background:rgba(43,214,123,.16);font-variant-numeric:tabular-nums}
    .num{font-family:var(--brand-font-display,Sora),sans-serif;font-size:clamp(26px,6.4vmin,56px);font-weight:800;letter-spacing:-0.025em;line-height:1.05;font-variant-numeric:tabular-nums}
    .sp{width:100%;height:5.5vh;margin-top:1.2vmin;overflow:visible}
    .sp path{fill:none;stroke:var(--brand-accent-secondary,#F7B733);stroke-width:3;stroke-linecap:round}
    .foot{display:inline-flex;align-items:center;gap:1.2vmin;justify-self:start;padding:1vmin 2vmin;border-radius:999px;font-size:clamp(12px,2.2vmin,16px);font-weight:600;background:rgba(255,255,255,.06);box-shadow:inset 0 0 0 1px rgba(255,255,255,.12)}
  </style>
  <div class="amb"><i id="orb0" style="width:65vw;height:45vh;left:35%;top:2%"></i><i id="orb1" style="width:70vw;height:45vh;left:-25%;top:55%;opacity:.26"></i></div>
  <div class="stage" id="rig">
    <div class="eyebrow" id="ey">VERIFIED RESULTS</div>
    <div class="grid">
      <div class="tile" id="t0"><span class="hero-ring" id="hr0"></span><div class="top"><span class="lbl">Revenue</span><span class="delta">+32%</span></div><div class="num" id="n0"></div><svg class="sp" viewBox="0 0 200 48"><path id="p0" d="M4,40 C40,36 72,30 110,22 C145,15 170,12 196,6"/></svg></div>
      <div class="tile" id="t1"><div class="top"><span class="lbl">Retention</span><span class="delta">+18%</span></div><div class="num" id="n1"></div><svg class="sp" viewBox="0 0 200 48"><path id="p1" d="M4,38 C45,35 80,28 120,24 C155,20 175,14 196,10"/></svg></div>
    </div>
    <div class="foot" id="ft">Quarterly cohort benchmark</div>
  </div>
  <script>
    const tl = gsap.timeline({ paused: true });
    tl.fromTo("#ey", { opacity: 0, y: 10 }, { opacity: 1, y: 0, duration: 0.30, ease: "power2.out" }, 0);
    const items = [
      { id: "#t0", n: "#n0", p: "#p0", to: 48.2, pre: "$", post: "K", dec: 1 },
      { id: "#t1", n: "#n1", p: "#p1", to: 94, pre: "", post: "%", dec: 0 }
    ];
    items.forEach((it, i) => {
      const at = 0.08 + i * 0.14;
      tl.fromTo(it.id, { y: 44, z: -220, rotationX: 14, opacity: 0 }, { y: 0, z: 0, rotationX: 0, opacity: 1, duration: 0.55, ease: "expo.out" }, at);
      tl.fromTo(it.id, { "--sheen": "-45%" }, { "--sheen": "145%", duration: 0.75, ease: "power2.inOut", immediateRender: false }, at + 0.22);
      const numEl = document.querySelector(it.n); const o = { v: 0 };
      tl.set(numEl, { opacity: 0 }, 0); tl.set(numEl, { opacity: 1 }, at + 0.16);
      tl.to(o, { v: it.to, duration: 0.85, ease: "power3.out", onUpdate: () => { numEl.textContent = it.pre + o.v.toFixed(it.dec) + it.post; } }, at + 0.16);
      tl.fromTo(numEl, { scale: 1.08 }, { scale: 1, duration: 0.25, ease: "expo.out", immediateRender: false }, at + 1.02);
      const path = document.querySelector(it.p); const L = path.getTotalLength();
      path.style.strokeDasharray = L; path.style.strokeDashoffset = L;
      tl.to(path, { strokeDashoffset: 0, duration: 0.85, ease: "power3.inOut" }, at + 0.20);
    });
    tl.fromTo("#ft", { y: 14, opacity: 0 }, { y: 0, opacity: 1, duration: 0.35, ease: "power2.out" }, 1.05);
    tl.to("#t0", { y: -10, scale: 1.04, duration: 0.45, ease: "back.out(1.8)" }, 1.25);
    tl.to("#hr0", { opacity: 1, duration: 0.35, ease: "power2.out" }, 1.25);
    tl.to("#t1", { opacity: 0.48, scale: 0.97, duration: 0.40, ease: "power2.inOut" }, 1.25);
    // Continuous subtle ambient drift through 4.0s while landed elements stay crisp
    tl.to("#orb0", { x: -30, y: 20, duration: 4.0, ease: "sine.inOut" }, 0);
    tl.to("#orb1", { x: 26, y: -18, duration: 4.0, ease: "sine.inOut" }, 0);
    window.__timelines = { main: tl };
  </script>
</div>
```

### Blueprint 2: Kinetic Slam Hook + Live Counter Pill (Frame-0 Ready)
Best for: `0.0–3.0s` opening hook with instant frame-0 ink, bold accent word, and living counter.

```html
<div class="clip" data-start="0" data-duration="3.0" style="position:fixed;inset:0;width:100vw;height:100vh;overflow:hidden;background:var(--brand-bg,#080404);color:var(--brand-ink,#F7F5EF);font-family:var(--brand-font-display,Sora),sans-serif;">
  <style>
    .bg{position:absolute;inset:0;background:radial-gradient(80% 55% at 50% 28%,color-mix(in srgb,var(--brand-accent,#F05A00) 28%,transparent),transparent 72%)}
    /* Use height: 56vh; padding: 6vh 5vw when T1 captions are active */
    .wrap{position:relative;width:100%;height:100vh;display:grid;grid-template-rows:auto 1fr auto;row-gap:2vh;align-items:center;padding:7vh 5vw 9vh;box-sizing:border-box}
    .pill{display:inline-flex;align-items:center;gap:1.4vmin;justify-self:start;padding:1.2vmin 2.4vmin;border-radius:999px;
      background:rgba(255,255,255,.08);box-shadow:inset 0 1px 0 rgba(255,255,255,.35),inset 0 0 0 1px rgba(255,255,255,.12);
      font-size:clamp(13px,2.6vmin,20px);font-weight:700;font-variant-numeric:tabular-nums}
    .dot{width:1.4vmin;height:1.4vmin;border-radius:50%;background:var(--brand-accent,#F05A00);box-shadow:0 0 14px var(--brand-accent,#F05A00)}
    .h1{font-size:clamp(36px,9.4vmin,88px);font-weight:800;letter-spacing:-0.025em;line-height:0.96}
    .lit{color:var(--brand-accent-secondary,#F7B733);text-shadow:0 0 28px color-mix(in srgb,var(--brand-accent,#F05A00) 60%,transparent)}
    .w{display:inline-block}
    .sub{font-family:var(--brand-font-body,Sora),sans-serif;font-size:clamp(14px,2.6vmin,20px);font-weight:600;opacity:.78}
  </style>
  <div class="bg"></div>
  <div class="wrap" id="rig">
    <div class="pill" id="pill"><span class="dot" id="dot"></span><span>LIVE IMPACT</span><span id="tc" style="opacity:.85"></span></div>
    <div class="h1" id="h1"><span class="w lit">10</span> <span class="w">hours</span><br/><span class="w">saved</span> <span class="w">weekly.</span></div>
    <div class="sub" id="sub">Automated workflow audit</div>
  </div>
  <script>
    const tl = gsap.timeline({ paused: true });
    // Frame 0 already has strong ink; headline settles cleanly by 0.35s
    tl.fromTo("#pill", { opacity: 0.88, scale: 1.04 }, { opacity: 1, scale: 1, duration: 0.35, ease: "expo.out" }, 0);
    tl.fromTo("#h1 .w", { opacity: 0, y: 20, scale: 1.25 }, { opacity: 1, y: 0, scale: 1, duration: 0.25, stagger: 0.03, ease: "expo.out" }, 0);
    const tc = document.querySelector("#tc"); const o = { v: 0 };
    tl.to(o, { v: 10, duration: 1.0, ease: "power3.out", onUpdate: () => { tc.textContent = "· " + Math.round(o.v) + " HRS"; } }, 0.12);
    tl.fromTo("#sub", { opacity: 0, y: 12 }, { opacity: 0.78, y: 0, duration: 0.32, ease: "power2.out" }, 0.85);
    [0.5, 1.0, 1.5, 2.0, 2.5].forEach((t) => {
      tl.fromTo("#dot", { scale: 1.45, opacity: 1 }, { scale: 1, opacity: 0.55, duration: 0.42, ease: "power2.out", immediateRender: false }, t);
    });
    window.__timelines = { main: tl };
  </script>
</div>
```

### Blueprint 3: Transparent Floating Strike-Correct / Verdict Overlay
Best for: myth-busting or sharp verdict over `V1` talking head (pair with `timeline.set_transform(track_id="g1", y=-0.24)`).

```html
<div class="clip" data-start="0" data-duration="3.0" style="display:grid;place-items:center;width:100vw;height:100vh;background:transparent;font-family:var(--brand-font-display,Sora),sans-serif;">
  <style>
    .card{position:relative;padding:20px 30px;border-radius:20px;max-width:82vw;text-align:center;
      background:color-mix(in srgb,var(--brand-surface,#1A0E0E) 92%,transparent);
      box-shadow:inset 0 1px 0 rgba(255,255,255,.3),inset 0 0 0 1px rgba(255,255,255,.12),0 24px 50px -18px rgba(0,0,0,.78);
      color:var(--brand-ink,#F7F5EF)}
    .old{position:relative;display:inline-block;font-size:clamp(20px,4.2vmin,32px);font-weight:700;opacity:.75}
    .strike{position:absolute;left:-4%;right:-4%;top:50%;height:4px;border-radius:4px;background:#FF5A5F;transform-origin:left center;transform:scaleX(0)}
    .new{margin-top:8px;font-size:clamp(24px,5.2vmin,42px);font-weight:800;letter-spacing:-0.025em;color:#2BD67B}
  </style>
  <div class="card" id="card">
    <div class="old">Post every day<span class="strike" id="st"></span></div>
    <div class="new" id="nw">Post 3 sharp hooks</div>
  </div>
  <script>
    const tl = gsap.timeline({ paused: true });
    tl.fromTo("#card", { y: 24, scale: 0.92, opacity: 0 }, { y: 0, scale: 1, opacity: 1, duration: 0.34, ease: "expo.out" }, 0);
    tl.to("#st", { scaleX: 1, duration: 0.28, ease: "power3.inOut" }, 0.58);
    tl.to(".old", { opacity: 0.42, duration: 0.24, ease: "power2.out" }, 0.65);
    // Keep back.out strictly on transforms; tween opacity with power2.out
    tl.fromTo("#nw", { y: 14, scale: 1.15 }, { y: 0, scale: 1, duration: 0.38, ease: "back.out(2.0)" }, 0.85);
    tl.fromTo("#nw", { opacity: 0 }, { opacity: 1, duration: 0.22, ease: "power2.out" }, 0.85);
    tl.to("#card", { y: -18, scale: 0.96, opacity: 0, duration: 0.30, ease: "power3.in", immediateRender: false }, 2.68);
    window.__timelines = { main: tl };
  </script>
</div>
```

### Blueprint 4: Bar Race + Winner Impact Stamp & Shockwave Ring
Best for: comparing options by one number where a clear leader overtakes and stamps.

```html
<div class="clip" data-start="0" data-duration="4.0" style="position:fixed;inset:0;width:100vw;height:100vh;overflow:hidden;background:var(--brand-bg,#080404);color:var(--brand-ink,#F7F5EF);font-family:var(--brand-font-body,Sora),sans-serif;">
  <style>
    .amb{position:absolute;inset:-20%;pointer-events:none;background:radial-gradient(55% 45% at 72% 35%,color-mix(in srgb,var(--brand-accent,#F05A00) 22%,transparent),transparent 70%)}
    /* Use height: 56vh; padding: 6vh 5vw when T1 captions are active */
    .stage{position:relative;width:100%;height:100vh;display:grid;grid-template-rows:auto 1fr auto;row-gap:2vh;align-items:center;padding:7vh 5vw 9vh;box-sizing:border-box}
    .hdr{font-family:var(--brand-font-display,Sora),sans-serif;font-size:clamp(22px,5vmin,40px);font-weight:800;letter-spacing:-0.02em}
    .rows{display:flex;flex-direction:column;gap:2.2vh;border-left:2px solid rgba(255,255,255,.22);padding-left:2.2vw;box-shadow:-12px 0 28px -18px var(--brand-accent,#F05A00)}
    .row{display:grid;grid-template-columns:1fr auto;align-items:center;gap:1.6vw}
    .track{position:relative;height:5.4vh;border-radius:12px;background:rgba(255,255,255,.06);overflow:hidden}
    .bar{position:relative;height:100%;border-radius:12px;transform-origin:left center;transform:scaleX(0);overflow:hidden;
      background:linear-gradient(90deg,rgba(255,255,255,.16),rgba(255,255,255,.28));box-shadow:inset 0 1px 0 rgba(255,255,255,.35)}
    .bar.win{background:linear-gradient(90deg,var(--brand-accent,#F05A00),var(--brand-accent-secondary,#F7B733))}
    .bar.win::after{content:"";position:absolute;inset:0;pointer-events:none;mix-blend-mode:screen;
      background:linear-gradient(110deg,transparent calc(var(--sheen,-45%) - 18%),rgba(255,255,255,.38) var(--sheen,-45%),transparent calc(var(--sheen,-45%) + 18%))}
    .lbl{position:absolute;left:1.6vmin;top:50%;transform:translateY(-50%);font-size:clamp(13px,2.4vmin,18px);font-weight:700;z-index:2}
    .val{font-size:clamp(15px,2.8vmin,22px);font-weight:800;font-variant-numeric:tabular-nums;min-width:4.5ch;text-align:right}
    .stamp-wrap{position:relative;justify-self:start;display:inline-grid;place-items:center}
    .stamp{padding:.75vmin 2vmin;border-radius:10px;border:2px solid #2BD67B;color:#2BD67B;font-size:clamp(13px,2.4vmin,18px);font-weight:800;letter-spacing:.12em;opacity:0}
    .ring{position:absolute;inset:-6px;border-radius:14px;border:2px solid #2BD67B;opacity:0;pointer-events:none}
  </style>
  <div class="amb" id="amb"></div>
  <div class="stage" id="rig">
    <div class="hdr" id="hdr">Render Speed Benchmark</div>
    <div class="rows">
      <div class="row"><div class="track"><span class="lbl">Solaris</span><div class="bar win" id="b0" style="width:94%"></div></div><span class="val" id="v0">3.2×</span></div>
      <div class="row"><div class="track"><span class="lbl">Legacy A</span><div class="bar" id="b1" style="width:52%"></div></div><span class="val" id="v1">1.4×</span></div>
      <div class="row"><div class="track"><span class="lbl">Legacy B</span><div class="bar" id="b2" style="width:36%"></div></div><span class="val" id="v2">1.0×</span></div>
    </div>
    <div class="stamp-wrap">
      <div class="ring" id="ring"></div>
      <div class="stamp" id="stamp">FASTEST · NO. 1</div>
    </div>
  </div>
  <script>
    const tl = gsap.timeline({ paused: true });
    tl.fromTo("#hdr", { opacity: 0, y: 12 }, { opacity: 1, y: 0, duration: 0.30, ease: "power2.out" }, 0);
    tl.to("#b2", { scaleX: 1, duration: 0.65, ease: "expo.out" }, 0.10);
    tl.to("#b1", { scaleX: 1, duration: 0.72, ease: "expo.out" }, 0.16);
    tl.to("#b0", { scaleX: 1, duration: 0.82, ease: "expo.out" }, 0.24);
    tl.fromTo("#v2, #v1, #v0", { opacity: 0, x: -8 }, { opacity: 1, x: 0, duration: 0.32, stagger: 0.08, ease: "power2.out" }, 0.55);
    // Impact stamp + shockwave ring + rig recoil + sheen sweep settled <= 1.75s
    tl.fromTo("#stamp", { scale: 2.2 }, { scale: 1, duration: 0.24, ease: "power4.in" }, 1.15);
    tl.fromTo("#stamp", { opacity: 0 }, { opacity: 1, duration: 0.16, ease: "power2.out" }, 1.15);
    tl.to("#stamp", { scale: 1.06, duration: 0.18, ease: "back.out(3)", immediateRender: false }, 1.39);
    tl.to("#stamp", { scale: 1, duration: 0.16, ease: "power2.out", immediateRender: false }, 1.57);
    tl.fromTo("#ring", { scale: 0.75, opacity: 0.85 }, { scale: 1.85, opacity: 0, duration: 0.55, ease: "power2.out" }, 1.38);
    tl.fromTo("#rig", { y: 4 }, { y: 0, duration: 0.28, ease: "back.out(2.2)", immediateRender: false }, 1.38);
    tl.fromTo("#b0", { "--sheen": "-45%" }, { "--sheen": "145%", duration: 0.75, ease: "power2.inOut", immediateRender: false }, 1.25);
    tl.to("#amb", { scale: 1.08, opacity: 0.85, duration: 4.0, ease: "sine.inOut" }, 0);
    window.__timelines = { main: tl };
  </script>
</div>
```

### Blueprint 5: AI Agent Tool-Call Trace / Terminal Run
Best for: showing an AI agent running a deterministic command, verifying steps, and landing `Done`.

```html
<div class="clip" data-start="0" data-duration="4.0" style="position:fixed;inset:0;width:100vw;height:100vh;overflow:hidden;background:var(--brand-bg,#080404);color:var(--brand-ink,#F7F5EF);font-family:var(--brand-font-body,Sora),sans-serif;">
  <style>
    /* Use height: 56vh; padding: 6vh 5vw when T1 captions are active */
    .stage{position:relative;width:100%;height:100vh;display:grid;grid-template-rows:auto 1fr auto;row-gap:2vh;align-items:center;padding:7vh 5vw 9vh;box-sizing:border-box}
    .hdr{font-family:var(--brand-font-display,Sora),sans-serif;font-size:clamp(20px,4.6vmin,36px);font-weight:800;letter-spacing:-0.02em}
    .term{padding:2.6vmin;border-radius:20px;background:rgba(255,255,255,.06);-webkit-backdrop-filter:blur(18px);backdrop-filter:blur(18px);
      box-shadow:inset 0 1px 0 rgba(255,255,255,.28),inset 0 0 0 1px rgba(255,255,255,.12),0 24px 55px -20px rgba(0,0,0,.7);display:flex;flex-direction:column;gap:1.8vh}
    .cmd{font-size:clamp(14px,2.5vmin,19px);font-weight:700;color:var(--brand-accent-secondary,#F7B733);font-variant-numeric:tabular-nums;min-height:1.4em}
    .step{display:flex;align-items:center;gap:1.4vmin;font-size:clamp(13px,2.3vmin,18px);font-weight:600;opacity:0}
    .ic{width:24px;height:24px;flex:0 0 24px}
    .foot{display:inline-flex;align-items:center;gap:1.2vmin;justify-self:start;padding:.9vmin 2vmin;border-radius:999px;background:rgba(43,214,123,.14);color:#2BD67B;font-size:clamp(13px,2.3vmin,18px);font-weight:800;opacity:0}
    .pulse{width:10px;height:10px;border-radius:50%;background:#2BD67B;box-shadow:0 0 12px #2BD67B}
  </style>
  <div class="stage">
    <div class="hdr" id="hdr">Agent Execution Trace</div>
    <div class="term" id="term">
      <div class="cmd" id="cmd"></div>
      <div class="step" id="s0">
        <svg class="ic" viewBox="0 0 24 24"><circle id="sp0" cx="12" cy="12" r="9" fill="none" stroke="var(--brand-accent,#F05A00)" stroke-width="2.5" stroke-dasharray="36 20"/><path id="ck0" d="M6 12.5 L10 16.5 L18 8.5" fill="none" stroke="#2BD67B" stroke-width="2.8" stroke-linecap="round" stroke-linejoin="round"/></svg>
        <span>assets.search("kpi-tiles")</span>
      </div>
      <div class="step" id="s1">
        <svg class="ic" viewBox="0 0 24 24"><circle id="sp1" cx="12" cy="12" r="9" fill="none" stroke="var(--brand-accent,#F05A00)" stroke-width="2.5" stroke-dasharray="36 20"/><path id="ck1" d="M6 12.5 L10 16.5 L18 8.5" fill="none" stroke="#2BD67B" stroke-width="2.8" stroke-linecap="round" stroke-linejoin="round"/></svg>
        <span>gfx.place(track="g1")</span>
      </div>
    </div>
    <div class="foot" id="badge"><span class="pulse" id="pulse"></span><span>Done · 0 lint errors</span></div>
  </div>
  <script>
    const tl = gsap.timeline({ paused: true });
    tl.fromTo("#hdr", { opacity: 0, y: 10 }, { opacity: 1, y: 0, duration: 0.28, ease: "power2.out" }, 0);
    tl.fromTo("#term", { opacity: 0, y: 20, scale: 0.97 }, { opacity: 1, y: 0, scale: 1, duration: 0.40, ease: "expo.out" }, 0.05);
    const cmdText = "$ solaris run --verify-gfx";
    const cmdEl = document.querySelector("#cmd");
    const typer = { n: 0 };
    tl.to(typer, { n: cmdText.length, duration: 0.55, ease: "none", onUpdate: () => { cmdEl.textContent = cmdText.slice(0, Math.round(typer.n)); } }, 0.12);
    ["0", "1"].forEach((idx, i) => {
      const ck = document.querySelector("#ck" + idx);
      const L = ck.getTotalLength();
      ck.style.strokeDasharray = L; ck.style.strokeDashoffset = L;
      const at = 0.65 + i * 0.35;
      tl.fromTo("#s" + idx, { opacity: 0, y: 8 }, { opacity: 1, y: 0, duration: 0.25, ease: "power2.out" }, at);
      tl.to("#sp" + idx, { rotation: 270, svgOrigin: "12 12", duration: 0.32, ease: "power1.inOut" }, at);
      tl.to("#sp" + idx, { opacity: 0, duration: 0.12, ease: "power2.out" }, at + 0.26);
      tl.to(ck, { strokeDashoffset: 0, duration: 0.24, ease: "power2.out" }, at + 0.28);
    });
    tl.fromTo("#badge", { scale: 0.85 }, { scale: 1, duration: 0.32, ease: "back.out(1.8)" }, 1.45);
    tl.fromTo("#badge", { opacity: 0 }, { opacity: 1, duration: 0.20, ease: "power2.out" }, 1.45);
    [1.9, 2.5, 3.1, 3.6].forEach((t) => {
      tl.fromTo("#pulse", { scale: 1.5, opacity: 1 }, { scale: 1, opacity: 0.55, duration: 0.45, ease: "power2.out", immediateRender: false }, t);
    });
    window.__timelines = { main: tl };
  </script>
</div>
```

### Blueprint 6: Procedural Editorial Paper Headline Highlight (12fps On-Twos + Conflict-Free Boil)
Best for: editorial/newsprint quote or headline proof with organic 12fps boil and chisel-marker highlight.

```html
<div class="clip" data-start="0" data-duration="3.5" style="position:fixed;inset:0;width:100vw;height:100vh;overflow:hidden;background:#141210;font-family:var(--brand-font-display,Sora),sans-serif;">
  <style>
    .grain{position:absolute;inset:0;width:100%;height:100%;opacity:.18;pointer-events:none}
    /* Use height: 56vh; padding: 6vh 5vw when T1 captions are active */
    .cam{position:relative;width:100%;height:100vh;display:grid;grid-template-rows:auto 1fr auto;row-gap:2vh;align-items:center;padding:7vh 5vw 9vh;box-sizing:border-box}
    .kicker{font-size:clamp(12px,2.2vmin,16px);font-weight:700;letter-spacing:.18em;color:#F5EFE2;opacity:.72}
    .paper{position:relative;padding:4vmin 3.8vmin;border-radius:8px;background:#F5EFE2;color:#141210;box-shadow:0 28px 65px -20px rgba(0,0,0,.85);
      translate:var(--bx,0px) var(--by,0px);rotate:var(--br,0deg)}
    .tape{position:absolute;top:-14px;left:50%;transform:translateX(-50%) rotate(-2deg);width:110px;height:28px;background:rgba(247,183,51,.55);box-shadow:0 2px 8px rgba(0,0,0,.18)}
    .hl-wrap{position:relative;display:inline;font-size:clamp(24px,5.6vmin,48px);font-weight:800;line-height:1.18;letter-spacing:-0.02em}
    .mark{position:relative;display:inline-block;padding:0 .16em}
    .mark::before{content:"";position:absolute;inset:8% -2% 4%;background:var(--brand-accent-secondary,#F7B733);mix-blend-mode:multiply;
      transform-origin:left center;transform:scaleX(var(--hl,0));border-radius:4px}
    .src{font-family:var(--brand-font-body,Sora),sans-serif;font-size:clamp(12px,2.2vmin,16px);font-weight:600;color:#F5EFE2;opacity:.7}
  </style>
  <svg class="grain" viewBox="0 0 200 200" preserveAspectRatio="none">
    <filter id="gf"><feTurbulence type="fractalNoise" baseFrequency="0.85" numOctaves="2" stitchTiles="stitch"/></filter>
    <rect width="100%" height="100%" filter="url(#gf)"/>
  </svg>
  <div class="cam" id="cam">
    <div class="kicker" id="kicker">EDITORIAL BRIEF</div>
    <div class="paper boil" id="paper">
      <div class="tape"></div>
      <div class="hl-wrap">Small teams ship <span class="mark" id="mark">3× faster</span> with automated QA.</div>
    </div>
    <div class="src" id="src">2026 Engineering Productivity Index</div>
  </div>
  <script>
    const tl = gsap.timeline({ paused: true });
    tl.fromTo("#kicker", { opacity: 0, y: 8 }, { opacity: 0.72, y: 0, duration: 0.28, ease: "power2.out" }, 0);
    tl.fromTo("#paper", { opacity: 0, y: 24, scale: 0.96 }, { opacity: 1, y: 0, scale: 1, duration: 0.42, ease: "expo.out" }, 0.05);
    // 12fps quantized chisel-marker sweep (steps(10) over ~0.8s)
    tl.to("#mark", { "--hl": 1, duration: 0.80, ease: "steps(10)" }, 0.45);
    tl.fromTo("#src", { opacity: 0, y: 10 }, { opacity: 0.7, y: 0, duration: 0.30, ease: "power2.out" }, 1.15);
    // Conflict-free 12fps boil via CSS translate/rotate vars while #cam pushes smoothly
    const boil = [{ x: "0px", y: "0px", r: "0deg" }, { x: "0.7px", y: "-0.6px", r: "0.22deg" }, { x: "-0.6px", y: "0.5px", r: "-0.18deg" }, { x: "0.4px", y: "0.4px", r: "0.14deg" }];
    for (let f = 0; f < 42; f++) {
      const b = boil[f % boil.length];
      tl.set("#paper", { "--bx": b.x, "--by": b.y, "--br": b.r }, f / 12);
    }
    tl.fromTo("#cam", { scale: 1 }, { scale: 1.05, duration: 3.5, ease: "power1.inOut" }, 0);
    window.__timelines = { main: tl };
  </script>
</div>
```

### Blueprint 7: Pure CSS 3D Extruded Listicle Ordinal Overlay
Best for: crisp listicle chapter/number callout over `V1` talking head (zero WebGL; pair with `timeline.set_transform(track_id="g1", y=-0.24)`).

```html
<div class="clip" data-start="0" data-duration="3.5" style="display:grid;place-items:center;width:100vw;height:100vh;background:transparent;font-family:var(--brand-font-display,Sora),sans-serif;">
  <style>
    .wrap{display:flex;align-items:center;gap:22px;padding:18px 28px;border-radius:22px;perspective:900px;
      background:color-mix(in srgb,var(--brand-surface,#1A0E0E) 90%,transparent);
      box-shadow:inset 0 1px 0 rgba(255,255,255,.32),inset 0 0 0 1px rgba(255,255,255,.14),0 24px 55px -18px rgba(0,0,0,.8)}
    .ext{position:relative;width:82px;height:82px;transform-style:preserve-3d;transform:rotateY(-18deg) rotateX(10deg)}
    .slice{position:absolute;inset:0;display:grid;place-items:center;font-size:58px;font-weight:800;line-height:1;color:color-mix(in srgb,var(--brand-accent,#F05A00) 45%,#000)}
    .face{position:relative;z-index:2;width:100%;height:100%;display:grid;place-items:center;font-size:58px;font-weight:800;line-height:1;
      color:var(--brand-ink,#F7F5EF);border-radius:16px;overflow:hidden;
      background:linear-gradient(145deg,var(--brand-accent,#F05A00),var(--brand-accent-secondary,#F7B733));
      box-shadow:inset 0 1px 0 rgba(255,255,255,.55)}
    .face::after{content:"";position:absolute;inset:0;mix-blend-mode:screen;
      background:linear-gradient(110deg,transparent calc(var(--sheen,-45%) - 18%),rgba(255,255,255,.45) var(--sheen,-45%),transparent calc(var(--sheen,-45%) + 18%))}
    .meta{display:flex;flex-direction:column;gap:8px}
    .rule{height:3px;width:48px;border-radius:3px;background:var(--brand-accent-secondary,#F7B733);box-shadow:0 0 12px var(--brand-accent,#F05A00);transform-origin:left center;transform:scaleX(0)}
    .title{font-size:clamp(22px,4.6vmin,36px);font-weight:800;color:var(--brand-ink,#F7F5EF);clip-path:inset(-40% 100% -40% -10%)}
  </style>
  <div class="wrap" id="card">
    <div class="ext" id="ext"><div class="face" id="face">03</div></div>
    <div class="meta">
      <div class="rule" id="rule"></div>
      <div class="title" id="title">Batch Your Hooks</div>
    </div>
  </div>
  <script>
    const ext = document.querySelector("#ext");
    for (let i = 1; i <= 16; i++) {
      const s = document.createElement("div");
      s.className = "slice";
      s.textContent = "03";
      s.style.transform = "translateZ(" + (-i * 1.5) + "px)";
      ext.insertBefore(s, ext.firstChild);
    }
    const tl = gsap.timeline({ paused: true });
    tl.fromTo("#card", { y: 24, scale: 0.92, opacity: 0 }, { y: 0, scale: 1, opacity: 1, duration: 0.35, ease: "expo.out" }, 0);
    tl.fromTo("#ext", { rotationY: -38, rotationX: 18 }, { rotationY: -16, rotationX: 8, duration: 0.55, ease: "expo.out" }, 0.08);
    tl.fromTo("#face", { "--sheen": "-45%" }, { "--sheen": "145%", duration: 0.70, ease: "power2.inOut", immediateRender: false }, 0.25);
    tl.to("#rule", { scaleX: 1, duration: 0.30, ease: "power3.out" }, 0.22);
    tl.to("#title", { clipPath: "inset(-40% -10% -40% -10%)", duration: 0.40, ease: "power3.out" }, 0.28);
    tl.to("#ext", { rotationY: -10, duration: 2.2, ease: "sine.inOut" }, 0.7);
    tl.to("#card", { y: -16, scale: 0.96, opacity: 0, duration: 0.30, ease: "power3.in", immediateRender: false }, 3.18);
    window.__timelines = { main: tl };
  </script>
</div>
```

### Blueprint 8: Feathered SVG Spotlight Dim + 3D Focus Lift
Best for: isolating one critical UI module by dimming surroundings with a feathered SVG spotlight mask.

```html
<div class="clip" data-start="0" data-duration="4.0" style="position:fixed;inset:0;width:100vw;height:100vh;overflow:hidden;background:var(--brand-bg,#080404);color:var(--brand-ink,#F7F5EF);font-family:var(--brand-font-body,Sora),sans-serif;">
  <style>
    /* Use height: 56vh; padding: 6vh 5vw when T1 captions are active */
    .stage{position:relative;width:100%;height:100vh;display:grid;grid-template-rows:auto 1fr auto;row-gap:2vh;align-items:center;padding:7vh 5vw 9vh;box-sizing:border-box;perspective:1400px}
    .hdr{font-family:var(--brand-font-display,Sora),sans-serif;font-size:clamp(22px,5vmin,38px);font-weight:800;letter-spacing:-0.02em;z-index:3}
    .board{position:relative;display:grid;grid-template-columns:repeat(2,1fr);gap:2vmin;transform-style:preserve-3d}
    .mod{padding:2.6vmin;border-radius:18px;background:rgba(255,255,255,.07);box-shadow:inset 0 0 0 1px rgba(255,255,255,.12);font-size:clamp(14px,2.5vmin,19px);font-weight:700}
    .mod.focal{position:relative;z-index:4;overflow:hidden;background:linear-gradient(155deg,rgba(255,255,255,.18),rgba(255,255,255,.06));
      box-shadow:inset 0 1px 0 rgba(255,255,255,.5),inset 0 0 0 2px color-mix(in srgb,var(--brand-accent,#F05A00) 75%,white),0 28px 65px -20px rgba(0,0,0,.8)}
    .mod.focal::after{content:"";position:absolute;inset:0;pointer-events:none;mix-blend-mode:screen;
      background:linear-gradient(110deg,transparent calc(var(--sheen,-45%) - 18%),rgba(255,255,255,.35) var(--sheen,-45%),transparent calc(var(--sheen,-45%) + 18%))}
    .spot-svg{position:absolute;inset:0;width:100%;height:100%;pointer-events:none;z-index:2}
    .pill{justify-self:start;z-index:3;padding:1vmin 2vmin;border-radius:999px;background:color-mix(in srgb,var(--brand-accent,#F05A00) 18%,transparent);box-shadow:inset 0 0 0 1px var(--brand-accent,#F05A00);font-size:clamp(13px,2.3vmin,17px);font-weight:700}
  </style>
  <div class="stage">
    <svg class="spot-svg" viewBox="0 0 1000 1000" preserveAspectRatio="none">
      <defs>
        <filter id="sf"><feGaussianBlur stdDeviation="36"/></filter>
        <mask id="sm">
          <rect width="1000" height="1000" fill="white"/>
          <circle id="hole" cx="720" cy="480" r="380" fill="black" filter="url(#sf)"/>
        </mask>
      </defs>
      <rect id="dim" width="1000" height="1000" fill="rgba(8,4,4,0)" mask="url(#sm)"/>
    </svg>
    <div class="hdr" id="hdr">Critical Bottleneck</div>
    <div class="board">
      <div class="mod">Ingest Queue</div>
      <div class="mod focal" id="focal">Export Worker · 98% Load</div>
      <div class="mod">Audio Mix</div>
      <div class="mod">Caption Sync</div>
    </div>
    <div class="pill" id="pill">Isolate worker pool to cut latency</div>
  </div>
  <script>
    const tl = gsap.timeline({ paused: true });
    const hole = document.querySelector("#hole");
    const dim = document.querySelector("#dim");
    const focal = document.querySelector("#focal");
    const pill = document.querySelector("#pill");
    const hdr = document.querySelector("#hdr");
    const clock = { t: 0 };
    tl.to(clock, {
      t: 1, duration: 4.0, ease: "none",
      onUpdate: () => {
        const t = clock.t;
        const enter = Math.min(1, t / 0.08);
        hdr.style.opacity = enter;
        const focus = Math.max(0, Math.min(1, (t - 0.12) / 0.24));
        const ease = 1 - Math.pow(1 - focus, 3);
        const pulse = focus >= 1 ? Math.sin(t * Math.PI * 2) * 6 : 0;
        hole.setAttribute("r", (380 - 190 * ease + pulse).toFixed(1));
        dim.setAttribute("fill", "rgba(8,4,4," + (0.76 * ease).toFixed(3) + ")");
        focal.style.transform = "translate3d(0," + (-12 * ease).toFixed(1) + "px," + (48 * ease).toFixed(1) + "px) rotateX(" + (4 * ease).toFixed(2) + "deg)";
        const sheen = -45 + Math.max(0, Math.min(1, (t - 0.18) / 0.20)) * 190;
        focal.style.setProperty("--sheen", sheen.toFixed(1) + "%");
        const pillIn = Math.max(0, Math.min(1, (t - 0.25) / 0.12));
        pill.style.opacity = pillIn;
        pill.style.transform = "translateY(" + ((1 - pillIn) * 12).toFixed(1) + "px)";
      }
    }, 0);
    window.__timelines = { main: tl };
  </script>
</div>
```
